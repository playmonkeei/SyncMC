//! Installs and verifies Mojang client files, optionally with a pinned Fabric profile.
use futures_util::{stream, StreamExt, TryStreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha1::{Digest, Sha1};
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{Emitter, Manager};

const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const DOWNLOAD_CONCURRENCY: usize = 12;
static CLIENT: OnceLock<Result<Client, String>> = OnceLock::new();
pub(crate) static INSTALL_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(crate) fn client() -> Result<&'static Client, String> {
    CLIENT
        .get_or_init(|| {
            Client::builder()
                .user_agent(concat!("SyncMC/", env!("CARGO_PKG_VERSION")))
                .https_only(true)
                .connect_timeout(Duration::from_secs(15))
                .timeout(Duration::from_secs(180))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| format!("Minecraft-Downloadclient konnte nicht starten: {e}"))
        })
        .as_ref()
        .map_err(Clone::clone)
}

fn official_url(url: &str) -> Result<reqwest::Url, String> {
    let parsed = reqwest::Url::parse(url).map_err(|_| "Ungültige Minecraft-Downloadadresse.")?;
    if parsed.scheme() != "https"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.port().is_some_and(|port| port != 443)
        || !matches!(
            parsed.host_str(),
            Some("piston-meta.mojang.com")
                | Some("piston-data.mojang.com")
                | Some("launchermeta.mojang.com")
                | Some("launcher.mojang.com")
                | Some("libraries.minecraft.net")
                | Some("resources.download.minecraft.net")
                | Some("maven.fabricmc.net")
                | Some("meta.fabricmc.net")
        )
    {
        return Err("Minecraft-Download verweist nicht auf einen offiziellen Server.".into());
    }
    Ok(parsed)
}

async fn manifest() -> Result<Value, String> {
    let response = client()?
        .get(MANIFEST_URL)
        .send()
        .await
        .map_err(|e| format!("Minecraft-Versionen konnten nicht geladen werden: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Minecraft-Versionen sind gerade nicht verfügbar: {e}"))?;
    response
        .json()
        .await
        .map_err(|_| "Das Minecraft-Versionsverzeichnis ist ungültig.".into())
}

#[tauri::command]
pub async fn get_minecraft_versions() -> Result<Value, String> {
    manifest().await
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallationReport {
    pub instance_id: String,
    pub version_id: String,
    #[serde(default = "vanilla_loader")]
    pub loader: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loader_version: Option<String>,
    pub directory: String,
    pub java_major_version: u64,
    pub files_verified: usize,
    pub bytes_verified: u64,
    pub native_archives: usize,
    pub installed_at: u64,
}

fn vanilla_loader() -> String {
    "vanilla".into()
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallProgress {
    instance_id: String,
    stage: &'static str,
    completed_files: usize,
    total_files: usize,
    downloaded_bytes: u64,
}

pub(crate) type ProgressCallback = Arc<dyn Fn(InstallProgress) + Send + Sync>;

pub(crate) fn valid_instance_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 100
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Err("Die Instanz-ID ist ungültig.".into());
    }
    Ok(())
}

pub(crate) fn relative_path(value: &str) -> Result<PathBuf, String> {
    let path = Path::new(value);
    if value.is_empty()
        || value.contains(['\\', ':', '\0'])
        || value.chars().any(char::is_control)
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        || value
            .split('/')
            .any(|s| s.is_empty() || s.ends_with(['.', ' ']))
    {
        return Err("Unsicherer Dateipfad in den Minecraft-Metadaten.".into());
    }
    Ok(path.to_path_buf())
}

/// Refuse symlinks inside our managed directory before creating or replacing a file.
pub(crate) fn managed_path(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    // Internal PathBufs use the host separator, including backslashes on Windows.
    // Validate components, while relative_path validates slash-separated API strings.
    if relative.as_os_str().is_empty()
        || relative.components().any(|component| match component {
            Component::Normal(part) => {
                let part = part.to_string_lossy();
                part.contains(['\\', ':', '\0'])
                    || part.chars().any(char::is_control)
                    || part.ends_with(['.', ' '])
            }
            _ => true,
        })
    {
        return Err("Unsicherer Dateipfad in den Minecraft-Metadaten.".into());
    }
    let mut path = root.to_path_buf();
    if fs::symlink_metadata(root).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("Der Minecraft-Installationsordner darf kein symbolischer Link sein.".into());
    }
    for part in relative.components() {
        path.push(part);
        if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err("Ein Minecraft-Installationspfad enthält einen symbolischen Link.".into());
        }
    }
    Ok(path)
}

fn prepare_file(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    let path = managed_path(root, relative)?;
    fs::create_dir_all(path.parent().ok_or("Ungültiger Installationspfad.")?)
        .map_err(|e| format!("Installationsordner konnte nicht erstellt werden: {e}"))?;
    Ok(path)
}

pub(crate) fn atomic_write(root: &Path, relative: &Path, bytes: &[u8]) -> Result<(), String> {
    let target = prepare_file(root, relative)?;
    let temp_relative = relative.with_extension("syncmc-part");
    let temp = prepare_file(root, &temp_relative)?;
    fs::write(&temp, bytes)
        .map_err(|e| format!("Minecraft-Datei konnte nicht geschrieben werden: {e}"))?;
    replace_file(&temp, &target)
}

fn replace_file(temp: &Path, target: &Path) -> Result<(), String> {
    // Windows cannot rename over an existing file. Only verified cache files are replaced here.
    #[cfg(target_os = "windows")]
    if target.exists() {
        fs::remove_file(target)
            .map_err(|e| format!("Minecraft-Datei konnte nicht ersetzt werden: {e}"))?;
    }
    fs::rename(temp, target)
        .map_err(|e| format!("Minecraft-Datei konnte nicht gespeichert werden: {e}"))
}

#[derive(Clone, Debug)]
pub(crate) struct Download {
    pub(crate) url: String,
    pub(crate) sha1: String,
    pub(crate) size: Option<u64>,
    pub(crate) path: PathBuf,
}

fn hash_valid(hash: &str) -> bool {
    hash.len() == 40 && hash.bytes().all(|c| c.is_ascii_hexdigit())
}

pub(crate) fn download_descriptor(value: &Value, path: PathBuf) -> Result<Download, String> {
    let url = value["url"]
        .as_str()
        .ok_or("Eine Minecraft-Downloadadresse fehlt.")?;
    official_url(url)?;
    let sha1 = value["sha1"]
        .as_str()
        .ok_or("Eine Minecraft-Prüfsumme fehlt.")?;
    if !hash_valid(sha1) {
        return Err("Ungültige Minecraft-SHA-1-Prüfsumme.".into());
    }
    Ok(Download {
        url: url.into(),
        sha1: sha1.to_ascii_lowercase(),
        size: value["size"].as_u64(),
        path,
    })
}

pub(crate) fn verify_file(path: &Path, expected: &Download) -> Result<bool, String> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(format!("Minecraft-Datei konnte nicht gelesen werden: {e}")),
    };
    if expected
        .size
        .is_some_and(|size| file.metadata().map_or(true, |m| m.len() != size))
    {
        return Ok(false);
    }
    let mut hash = Sha1::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|e| format!("Prüfsummenprüfung fehlgeschlagen: {e}"))?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hash.finalize()) == expected.sha1)
}

pub(crate) async fn download(root: &Path, item: &Download) -> Result<u64, String> {
    let path = prepare_file(root, &item.path)?;
    if verify_file(&path, item)? {
        return Ok(0);
    }
    let temp = prepare_file(root, &item.path.with_extension("syncmc-part"))?;
    let mut last_error = String::new();
    for attempt in 0..3 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(250 * attempt)).await;
        }
        let result = async {
            let mut response = client()?
                .get(official_url(&item.url)?)
                .send()
                .await
                .map_err(|e| format!("Download fehlgeschlagen: {e}"))?
                .error_for_status()
                .map_err(|e| format!("Download fehlgeschlagen: {e}"))?;
            let mut file = File::create(&temp)
                .map_err(|e| format!("Downloaddatei konnte nicht erstellt werden: {e}"))?;
            let mut hash = Sha1::new();
            let mut written = 0_u64;
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|e| format!("Download unterbrochen: {e}"))?
            {
                written += chunk.len() as u64;
                if written > item.size.unwrap_or(512 * 1024 * 1024) {
                    return Err("Minecraft-Datei ist größer als angekündigt.".to_owned());
                }
                hash.update(&chunk);
                file.write_all(&chunk)
                    .map_err(|e| format!("Download konnte nicht gespeichert werden: {e}"))?;
            }
            if item.size.is_some_and(|size| size != written)
                || format!("{:x}", hash.finalize()) != item.sha1
            {
                return Err(
                    "Minecraft-Prüfsumme stimmt nicht überein. Die Datei wird erneut geladen."
                        .to_owned(),
                );
            }
            file.sync_all()
                .map_err(|e| format!("Minecraft-Datei konnte nicht gesichert werden: {e}"))?;
            drop(file);
            replace_file(&temp, &path)?;
            Ok(written)
        }
        .await;
        match result {
            Ok(bytes) => return Ok(bytes),
            Err(error) => {
                last_error = error;
                let _ = fs::remove_file(&temp);
            }
        }
    }
    Err(format!("{}: {last_error}", item.path.display()))
}

pub(crate) fn minecraft_os() -> &'static str {
    match std::env::consts::OS {
        "macos" => "osx",
        "windows" => "windows",
        _ => "linux",
    }
}

fn regex_matches(pattern: &str, value: &str) -> bool {
    regex::Regex::new(pattern).is_ok_and(|r| r.is_match(value))
}

pub(crate) fn os_arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "aarch64",
        "x86" => "x86",
        _ => "x86_64",
    }
}

pub(crate) fn rules_allow(rules: Option<&Value>, os: &str, arch: &str, os_version: &str) -> bool {
    let Some(rules) = rules.and_then(Value::as_array) else {
        return true;
    };
    let mut allowed = false;
    for rule in rules {
        let matching_os = rule.get("os").is_none_or(|o| {
            o.get("name")
                .and_then(Value::as_str)
                .is_none_or(|name| name == os)
                && o.get("arch")
                    .and_then(Value::as_str)
                    .is_none_or(|p| regex_matches(p, arch))
                && o.get("version")
                    .and_then(Value::as_str)
                    .is_none_or(|p| regex_matches(p, os_version))
        });
        // This installer enables no special/demo/custom-resolution feature flags.
        let matching_features = rule
            .get("features")
            .and_then(Value::as_object)
            .is_none_or(|features| features.values().all(|v| v.as_bool() == Some(false)));
        if matching_os && matching_features {
            allowed = rule["action"] == "allow";
        }
    }
    allowed
}

/// New manifests encode natives as ordinary artifacts, often without architecture rules.
pub(crate) fn native_matches_arch(name: &str, arch: &str) -> bool {
    let Some(classifier) = name.split(':').nth(3) else {
        return true;
    };
    if !classifier.starts_with("natives-") {
        return true;
    }
    let arm = classifier.contains("arm64") || classifier.contains("aarch64");
    let arm32 = classifier.contains("arm32");
    let x86 = classifier.ends_with("-x86");
    if arch == "aarch64" {
        arm
    } else if arch == "x86" {
        x86 || (!arm && !arm32 && classifier == "natives-windows")
    } else {
        !arm && !arm32 && !x86
    }
}

pub(crate) fn host_version() -> String {
    #[cfg(unix)]
    {
        std::process::Command::new("uname")
            .arg("-r")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
            .unwrap_or_default()
    }
    #[cfg(not(unix))]
    {
        String::new()
    }
}

pub(crate) fn library_downloads(
    version: &Value,
    os: &str,
    arch: &str,
    os_version: &str,
) -> Result<(Vec<Download>, Vec<(PathBuf, Vec<String>)>), String> {
    let mut downloads = Vec::new();
    let mut natives = Vec::new();
    for library in version["libraries"]
        .as_array()
        .ok_or("Minecraft-Bibliotheken fehlen.")?
    {
        if !rules_allow(library.get("rules"), os, arch, os_version) {
            continue;
        }
        let name = library["name"]
            .as_str()
            .ok_or("Ein Bibliotheksname fehlt.")?;
        if !native_matches_arch(name, arch) {
            continue;
        }
        let excludes = library["extract"]["exclude"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_else(|| vec!["META-INF/".into()]);
        if let Some(artifact) = library["downloads"].get("artifact") {
            let path = PathBuf::from("libraries").join(relative_path(
                artifact["path"].as_str().ok_or("Bibliothekspfad fehlt.")?,
            )?);
            if name
                .split(':')
                .nth(3)
                .is_some_and(|c| c.starts_with("natives-"))
            {
                natives.push((path.clone(), excludes.clone()));
            }
            downloads.push(download_descriptor(artifact, path)?);
        }
        if let Some(classifier) = library["natives"][os].as_str() {
            let classifier = classifier.replace("${arch}", if arch == "x86" { "32" } else { "64" });
            let artifact = library["downloads"]["classifiers"]
                .get(&classifier)
                .ok_or("Native Minecraft-Bibliothek fehlt.")?;
            let path = PathBuf::from("libraries").join(relative_path(
                artifact["path"]
                    .as_str()
                    .ok_or("Nativer Bibliothekspfad fehlt.")?,
            )?);
            natives.push((path.clone(), excludes));
            downloads.push(download_descriptor(artifact, path)?);
        }
        if library["downloads"].get("artifact").is_none() && library.get("natives").is_none() {
            return Err(format!(
                "Für die Bibliothek {name} fehlen verifizierbare Downloads."
            ));
        }
    }
    Ok((downloads, natives))
}

pub(crate) fn extract_natives(
    root: &Path,
    archives: &[(PathBuf, Vec<String>)],
    destination: &Path,
) -> Result<(), String> {
    for (archive, excludes) in archives {
        let file = File::open(managed_path(root, archive)?)
            .map_err(|e| format!("Native Bibliothek konnte nicht geöffnet werden: {e}"))?;
        let mut zip = zip::ZipArchive::new(file)
            .map_err(|e| format!("Native Bibliothek ist kein gültiges Archiv: {e}"))?;
        for i in 0..zip.len() {
            let mut entry = zip
                .by_index(i)
                .map_err(|e| format!("Native Bibliothek konnte nicht gelesen werden: {e}"))?;
            if entry.is_dir()
                || excludes
                    .iter()
                    .any(|prefix| entry.name().starts_with(prefix))
                || entry.name().starts_with("META-INF/")
            {
                continue;
            }
            if entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
                return Err("Symbolischer Link im Minecraft-Archiv abgelehnt.".into());
            }
            let relative = relative_path(entry.name())?;
            // Modern artifacts also contain Java classes; keep only native binaries.
            if ![".dylib", ".jnilib", ".so", ".dll"]
                .iter()
                .any(|extension| entry.name().ends_with(extension))
            {
                continue;
            }
            if entry.size() > 128 * 1024 * 1024 {
                return Err("Native Bibliothek ist zu groß.".into());
            }
            let path = prepare_file(root, &destination.join(relative))?;
            let mut target = File::create(path)
                .map_err(|e| format!("Native Bibliothek konnte nicht geschrieben werden: {e}"))?;
            std::io::copy(&mut entry, &mut target)
                .map_err(|e| format!("Native Bibliothek konnte nicht entpackt werden: {e}"))?;
        }
    }
    Ok(())
}

pub(crate) fn report_path(instance_id: &str) -> PathBuf {
    PathBuf::from("instances")
        .join(instance_id)
        .join("installation.json")
}

pub async fn install_at(
    root: &Path,
    instance_id: &str,
    version_id: &str,
    loader: &str,
    loader_version: Option<&str>,
    progress: ProgressCallback,
) -> Result<InstallationReport, String> {
    valid_instance_id(instance_id)?;
    if !matches!(loader, "vanilla" | "fabric") {
        return Err(
            "Die Installation unterstützt Vanilla und Fabric. Forge und NeoForge folgen später."
                .into(),
        );
    }
    // Even official IDs are checked before being used in any local path.
    let version_component = relative_path(version_id)?;
    if version_component.components().count() != 1 || version_id.len() > 100 {
        return Err("Ungültige Minecraft-Version.".into());
    }
    fs::create_dir_all(root)
        .map_err(|e| format!("Minecraft-Installationsordner konnte nicht erstellt werden: {e}"))?;
    let report_file = managed_path(root, &report_path(instance_id))?;
    if report_file.exists() {
        fs::remove_file(&report_file)
            .map_err(|e| format!("Installationsstatus konnte nicht aktualisiert werden: {e}"))?;
    }
    let state = Arc::new(Mutex::new(InstallProgress {
        instance_id: instance_id.into(),
        stage: "metadata",
        completed_files: 0,
        total_files: 0,
        downloaded_bytes: 0,
    }));
    progress(state.lock().unwrap().clone());
    let manifest = manifest().await?;
    let entry = manifest["versions"]
        .as_array()
        .and_then(|v| {
            v.iter().find(|v| {
                v["id"] == version_id && matches!(v["type"].as_str(), Some("release" | "snapshot"))
            })
        })
        .ok_or("Diese Minecraft-Version wurde von Mojang nicht veröffentlicht.")?;
    let version_relative = PathBuf::from("versions").join(&version_component);
    let metadata = download_descriptor(entry, version_relative.join(format!("{version_id}.json")))?;
    let metadata_downloaded = download(root, &metadata).await?;
    let version: Value = serde_json::from_slice(
        &fs::read(managed_path(root, &metadata.path)?).map_err(|e| e.to_string())?,
    )
    .map_err(|_| "Minecraft-Versionsdatei ist ungültig.")?;
    if version["id"] != version_id || version["mainClass"].as_str().is_none() {
        return Err("Minecraft-Versionsdatei passt nicht zur Auswahl.".into());
    }
    let assets_id = version["assetIndex"]["id"]
        .as_str()
        .ok_or("Minecraft-Assetindex fehlt.")?;
    let assets_component = relative_path(assets_id)?;
    if assets_component.components().count() != 1 {
        return Err("Ungültiger Minecraft-Assetindex.".into());
    }
    let index = download_descriptor(
        &version["assetIndex"],
        PathBuf::from("assets/indexes").join(format!("{assets_id}.json")),
    )?;
    let index_downloaded = download(root, &index).await?;
    let asset_index: Value = serde_json::from_slice(
        &fs::read(managed_path(root, &index.path)?).map_err(|e| e.to_string())?,
    )
    .map_err(|_| "Minecraft-Assetindex ist ungültig.")?;
    let (mut downloads, native_archives) =
        library_downloads(&version, minecraft_os(), os_arch(), &host_version())?;
    downloads.push(download_descriptor(
        &version["downloads"]["client"],
        version_relative.join(format!("{version_id}.jar")),
    )?);
    if let Some(logging) = version["logging"]["client"].get("file") {
        let id = relative_path(
            logging["id"]
                .as_str()
                .ok_or("Minecraft-Loggingdatei fehlt.")?,
        )?;
        downloads.push(download_descriptor(
            logging,
            PathBuf::from("assets/log_configs").join(id),
        )?);
    }
    let assets = asset_index["objects"]
        .as_object()
        .ok_or("Minecraft-Assetobjekte fehlen.")?;
    let mut unique_assets = HashSet::new();
    for (name, asset) in assets {
        relative_path(name)?;
        let hash = asset["hash"]
            .as_str()
            .filter(|hash| hash_valid(hash))
            .ok_or("Ungültige Minecraft-Assetprüfsumme.")?
            .to_ascii_lowercase();
        if unique_assets.insert(hash.clone()) {
            downloads.push(Download {
                url: format!(
                    "https://resources.download.minecraft.net/{}/{hash}",
                    &hash[..2]
                ),
                sha1: hash.clone(),
                size: Some(
                    asset["size"]
                        .as_u64()
                        .ok_or("Minecraft-Assetgröße fehlt.")?,
                ),
                path: PathBuf::from("assets/objects").join(&hash[..2]).join(hash),
            });
        }
    }
    // Shared artifacts can occur more than once in a manifest. Never race their temporary files.
    let mut by_path = BTreeMap::new();
    for item in downloads {
        by_path.insert(item.path.clone(), item);
    }
    let downloads: Vec<Download> = by_path.into_values().collect();
    {
        let mut current = state.lock().unwrap();
        current.stage = "downloading";
        current.total_files = downloads.len() + 2;
        current.completed_files = 2;
        current.downloaded_bytes = metadata_downloaded + index_downloaded;
        progress(current.clone());
    }
    stream::iter(downloads.clone().into_iter().map(|item| {
        let state = state.clone();
        let progress = progress.clone();
        async move {
            let bytes = download(root, &item).await?;
            let mut current = state.lock().unwrap();
            current.completed_files += 1;
            current.downloaded_bytes += bytes;
            // At most ~100 updates per install, plus the first downloads and completion.
            if current.completed_files <= 5
                || current.completed_files % 50 == 0
                || current.completed_files == current.total_files
            {
                progress(current.clone());
            }
            Ok::<(), String>(())
        }
    }))
    .buffer_unordered(DOWNLOAD_CONCURRENCY)
    .try_collect::<Vec<_>>()
    .await?;
    {
        let mut current = state.lock().unwrap();
        current.stage = "natives";
        progress(current.clone());
    }
    let instance_relative = PathBuf::from("instances").join(instance_id);
    let natives_relative = instance_relative.join("natives").join(&version_component);
    extract_natives(root, &native_archives, &natives_relative)?;
    // Older versions look up assets by logical name instead of the content-addressed object tree.
    for (name, asset) in assets {
        let hash = asset["hash"].as_str().unwrap();
        let source = managed_path(
            root,
            &PathBuf::from("assets/objects").join(&hash[..2]).join(hash),
        )?;
        let mut targets = Vec::new();
        if asset_index["virtual"] == true {
            targets.push(
                PathBuf::from("assets/virtual")
                    .join(&assets_component)
                    .join(relative_path(name)?),
            );
        }
        if asset_index["map_to_resources"] == true {
            targets.push(
                instance_relative
                    .join("resources")
                    .join(relative_path(name)?),
            );
        }
        for relative in targets {
            let target = prepare_file(root, &relative)?;
            fs::copy(&source, target)
                .map_err(|e| format!("Minecraft-Assets konnten nicht zugeordnet werden: {e}"))?;
        }
    }
    let (version, pinned_loader, extra_files, extra_bytes) = if loader == "fabric" {
        {
            let mut current = state.lock().unwrap();
            current.stage = "loader";
            progress(current.clone());
        }
        let installed = crate::fabric::install(root, version_id, loader_version, version).await?;
        (
            installed.version,
            Some(installed.loader_version),
            installed.files,
            installed.bytes,
        )
    } else {
        (version, None, 0, 0)
    };
    // Persist the resolved launch inputs separately from the syncable instance profile.
    atomic_write(
        root,
        &instance_relative.join("resolved-version.json"),
        &serde_json::to_vec_pretty(&version).map_err(|e| e.to_string())?,
    )?;
    let report = InstallationReport {
        instance_id: instance_id.into(),
        version_id: version_id.into(),
        loader: loader.into(),
        loader_version: pinned_loader,
        directory: root.join(&instance_relative).to_string_lossy().into(),
        java_major_version: version["javaVersion"]["majorVersion"].as_u64().unwrap_or(8),
        files_verified: downloads.len() + 2 + extra_files,
        bytes_verified: downloads.iter().map(|d| d.size.unwrap_or(0)).sum::<u64>()
            + index.size.unwrap_or(0)
            + extra_bytes
            + fs::metadata(managed_path(root, &metadata.path)?)
                .map_err(|e| e.to_string())?
                .len(),
        native_archives: native_archives.len(),
        installed_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs(),
    };
    atomic_write(
        root,
        &report_path(instance_id),
        &serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
    )?;
    {
        let mut current = state.lock().unwrap();
        current.stage = "complete";
        progress(current.clone());
    }
    Ok(report)
}

#[tauri::command]
pub async fn install_minecraft(
    app: tauri::AppHandle,
    instance_id: String,
    version_id: String,
    loader: String,
    loader_version: Option<String>,
) -> Result<InstallationReport, String> {
    let _guard = INSTALL_LOCK
        .try_lock()
        .map_err(|_| "Es läuft bereits eine Minecraft-Installation. Bitte warten.")?;
    crate::launch::ensure_stopped()?;
    let root = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("App-Datenordner konnte nicht ermittelt werden: {e}"))?
        .join("minecraft");
    let progress: ProgressCallback = Arc::new(move |progress| {
        let _ = app.emit("minecraft-install-progress", progress);
    });
    install_at(
        &root,
        &instance_id,
        &version_id,
        &loader,
        loader_version.as_deref(),
        progress,
    )
    .await
}

#[tauri::command]
pub async fn get_minecraft_installation(
    app: tauri::AppHandle,
    instance_id: String,
) -> Result<Option<InstallationReport>, String> {
    valid_instance_id(&instance_id)?;
    let root = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("minecraft");
    let path = managed_path(&root, &report_path(&instance_id))?;
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(format!(
                "Installationsstatus konnte nicht gelesen werden: {e}"
            ))
        }
    };
    let report: InstallationReport = serde_json::from_slice(&bytes)
        .map_err(|_| "Installationsstatus ist beschädigt. Bitte die Installation erneut prüfen.")?;
    if report.instance_id != instance_id {
        return Err("Installationsstatus passt nicht zur Instanz.".into());
    }
    Ok(Some(report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rejects_paths_outside_managed_directory() {
        for path in [
            "../escape",
            "/tmp/escape",
            "a/../../b",
            "C:/escape",
            "a\\escape",
            "a//b",
            ".",
            "a.",
            "a ",
        ] {
            assert!(relative_path(path).is_err(), "{path}");
        }
        assert!(relative_path("assets/objects/ab/abcdef").is_ok());
        assert!(relative_path("1.14 Pre-Release 5").is_ok());
        assert!(valid_instance_id("../../evil").is_err());
    }

    #[test]
    fn only_trusts_official_https_downloads() {
        for url in [
            "http://libraries.minecraft.net/a",
            "https://evil.example/a",
            "https://libraries.minecraft.net.evil.example/a",
            "https://user@libraries.minecraft.net/a",
            "https://libraries.minecraft.net:444/a",
        ] {
            assert!(official_url(url).is_err(), "{url}");
        }
        assert!(official_url(MANIFEST_URL).is_ok());
    }

    #[test]
    fn rule_order_and_architecture_are_respected() {
        let rules =
            json!([{ "action": "allow" }, { "action": "disallow", "os": { "name": "osx" } }]);
        assert!(!rules_allow(Some(&rules), "osx", "aarch64", "25.0"));
        assert!(rules_allow(Some(&rules), "linux", "x86_64", "6.1"));
        assert!(native_matches_arch(
            "org.lwjgl:lwjgl:3.4.3:natives-macos-arm64",
            "aarch64"
        ));
        assert!(!native_matches_arch(
            "org.lwjgl:lwjgl:3.4.3:natives-macos",
            "aarch64"
        ));
        assert!(!native_matches_arch(
            "org.lwjgl:lwjgl:3.4.3:natives-macos-arm64",
            "x86_64"
        ));
        let feature = json!([{ "action": "allow", "features": { "is_demo_user": true } }]);
        assert!(!rules_allow(Some(&feature), "osx", "aarch64", "25.0"));
    }

    #[test]
    fn verifies_and_rejects_corrupt_files() {
        let dir = std::env::temp_dir().join(format!("syncmc-checksum-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("sample");
        let expected = Download {
            url: String::new(),
            sha1: "a9993e364706816aba3e25717850c26c9cd0d89d".into(),
            size: Some(3),
            path: "sample".into(),
        };
        fs::write(&file, b"abc").unwrap();
        assert!(verify_file(&file, &expected).unwrap());
        fs::write(&file, b"abd").unwrap();
        assert!(!verify_file(&file, &expected).unwrap());
        fs::write(&file, b"abcde").unwrap();
        assert!(!verify_file(&file, &expected).unwrap());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[ignore = "Downloads the real latest snapshot and verifies all published Minecraft files (~650 MB)."]
    fn real_minecraft_installation() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let manifest = manifest().await.unwrap();
            let version = std::env::var("SYNCMC_TEST_VERSION")
                .unwrap_or_else(|_| manifest["latest"]["snapshot"].as_str().unwrap().into());
            let root = std::env::var_os("SYNCMC_INSTALL_TEST_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/private/tmp/syncmc-install-smoke"));
            let callback: ProgressCallback = Arc::new(|p| {
                eprintln!(
                    "{}: {}/{} files; {} bytes downloaded",
                    p.stage, p.completed_files, p.total_files, p.downloaded_bytes
                )
            });
            let report = install_at(
                &root,
                "snapshot-smoke",
                &version,
                "vanilla",
                None,
                callback.clone(),
            )
            .await
            .unwrap();
            assert!(report.files_verified > 1000);
            assert!(report.bytes_verified > 100_000_000);
            assert!(report.native_archives > 0);
            assert!(root.join(report_path("snapshot-smoke")).is_file());
            let mut assets = fs::read_dir(root.join("assets/objects"))
                .unwrap()
                .flat_map(|prefix| fs::read_dir(prefix.unwrap().path()).unwrap())
                .map(|f| f.unwrap().path());
            let damaged = assets.next().unwrap();
            fs::write(&damaged, b"deliberately corrupt test asset").unwrap();
            let repaired = install_at(&root, "snapshot-smoke", &version, "vanilla", None, callback)
                .await
                .unwrap();
            assert_eq!(report.files_verified, repaired.files_verified);
            let hash = damaged.file_name().unwrap().to_string_lossy().into_owned();
            assert!(verify_file(
                &damaged,
                &Download {
                    url: String::new(),
                    sha1: hash,
                    size: None,
                    path: PathBuf::new()
                }
            )
            .unwrap());
            eprintln!(
                "INSTALLATION_REPORT {}",
                serde_json::to_string_pretty(&repaired).unwrap()
            );
        });
    }
}

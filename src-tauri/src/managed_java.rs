//! App-local Eclipse Temurin runtimes. No system installation or environment changes.
//! Sources: https://adoptium.net/installation/ci-scripts and https://api.adoptium.net/q/openapi

use crate::runtime::{self, RuntimeInfo};
use flate2::read::GzDecoder;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tauri::{Emitter, Manager};
use tokio::sync::Mutex;

const PROOF: &str = ".syncmc-java.json";
const MAX_ARCHIVE: u64 = 512 * 1024 * 1024;
const MAX_EXPANDED: u64 = 3 * 1024 * 1024 * 1024;
const MAX_FILES: usize = 40_000;
static INSTALL_LOCK: Mutex<()> = Mutex::const_new(());

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaProgress {
    pub major: u32,
    pub stage: &'static str,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
}
pub type Progress = Arc<dyn Fn(JavaProgress) + Send + Sync>;

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Package {
    checksum: String,
    size: u64,
    link: String,
    name: String,
}
#[derive(Deserialize)]
struct Asset {
    binary: Binary,
    vendor: String,
    release_name: String,
    version: Version,
}
#[derive(Deserialize)]
struct Version {
    major: u32,
}
#[derive(Deserialize)]
struct Binary {
    architecture: String,
    os: String,
    image_type: String,
    jvm_impl: String,
    heap_size: String,
    package: Package,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
struct FileProof {
    path: String,
    sha256: String,
    size: u64,
    executable: bool,
}
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
struct LinkProof {
    path: String,
    target: String,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeProof {
    format: String,
    version: u32,
    major: u32,
    os: String,
    arch: String,
    release: String,
    package: Package,
    executable: String,
    files: Vec<FileProof>,
    links: Vec<LinkProof>,
}

fn host() -> Result<(&'static str, &'static str), String> {
    let os = match std::env::consts::OS {
        "macos" => "mac",
        "windows" => "windows",
        "linux" => "linux",
        _ => {
            return Err(
                "Die automatische Java-Einrichtung unterstützt dieses Betriebssystem nicht.".into(),
            )
        }
    };
    let arch =
        match std::env::consts::ARCH {
            "aarch64" => "aarch64",
            "x86_64" => "x64",
            "x86" => "x86",
            _ => return Err(
                "Für diese Prozessorarchitektur ist keine automatische Java-Einrichtung verfügbar."
                    .into(),
            ),
        };
    Ok((os, arch))
}

fn supported_major(major: u32) -> Result<(), String> {
    if !matches!(major, 8 | 11 | 16 | 17 | 21 | 25) {
        return Err(format!("Java {major} wird noch nicht automatisch eingerichtet. Bitte einen eigenen Java-Pfad auswählen."));
    }
    Ok(())
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn safe_component(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.len() > 255
        || value.contains(['/', '\\', ':', '\0', '<', '>', '"', '|', '?', '*'])
        || value.chars().any(char::is_control)
        || value.ends_with(['.', ' '])
    {
        return Err("Unsicherer Dateipfad im Java-Archiv.".into());
    }
    let stem = value.split('.').next().unwrap_or("").to_ascii_lowercase();
    if matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || (stem.len() == 4
            && (stem.starts_with("com") || stem.starts_with("lpt"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
    {
        return Err("Reservierter Dateiname im Java-Archiv.".into());
    }
    Ok(())
}

fn wire(path: &Path) -> Result<String, String> {
    let mut result = Vec::new();
    for component in path.components() {
        let Component::Normal(part) = component else {
            return Err("Unsicherer Dateipfad im Java-Archiv.".into());
        };
        let part = part
            .to_str()
            .ok_or("Ungültiger Dateiname im Java-Archiv.")?;
        safe_component(part)?;
        result.push(part);
    }
    if result.is_empty() {
        return Err("Leerer Dateipfad im Java-Archiv.".into());
    }
    Ok(result.join("/"))
}

fn relative(value: &str) -> Result<PathBuf, String> {
    if value.len() > 2_048 || value.starts_with('/') || value.contains('\\') {
        return Err("Unsicherer Dateipfad im Java-Archiv.".into());
    }
    let path = Path::new(value);
    wire(path)?;
    Ok(path.to_path_buf())
}

fn managed_path(root: &Path, path: &Path, allow_leaf_link: bool) -> Result<PathBuf, String> {
    wire(path)?;
    if fs::symlink_metadata(root).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("Der Java-Datenordner darf kein symbolischer Link sein.".into());
    }
    let parts: Vec<_> = path.components().collect();
    let mut target = root.to_path_buf();
    for (index, part) in parts.iter().enumerate() {
        target.push(part);
        if (!allow_leaf_link || index + 1 < parts.len())
            && fs::symlink_metadata(&target).is_ok_and(|m| m.file_type().is_symlink())
        {
            return Err("Ein verwalteter Java-Pfad enthält einen symbolischen Link.".into());
        }
    }
    Ok(target)
}

fn root(app_data: &Path) -> Result<PathBuf, String> {
    managed_path(app_data, Path::new("java-runtimes"), false)
}

fn slot(major: u32, os: &str, arch: &str) -> PathBuf {
    PathBuf::from(format!("temurin-{major}-{os}-{arch}"))
}

fn destination(url: &Url, major: u32, initial: bool) -> bool {
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some_and(|p| p != 443)
        || url.fragment().is_some()
    {
        return false;
    }
    if url.host_str() == Some("github.com") {
        return url.path().starts_with(&format!(
            "/adoptium/temurin{major}-binaries/releases/download/"
        )) && url.query().is_none();
    }
    !initial
        && matches!(
            url.host_str(),
            Some(
                "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
                    | "github-releases.githubusercontent.com"
            )
        )
}

fn check_package(
    package: &Package,
    major: u32,
    os: &str,
    arch: &str,
    image: &str,
) -> Result<(), String> {
    safe_component(&package.name)?;
    let extension = if os == "windows" { ".zip" } else { ".tar.gz" };
    if !package
        .name
        .starts_with(&format!("OpenJDK{major}U-{image}_{arch}_{os}_hotspot_"))
        || !package.name.ends_with(extension)
        || !valid_hash(&package.checksum)
        || package.size == 0
        || package.size > MAX_ARCHIVE
    {
        return Err("Adoptium hat kein gültiges, passendes Java-Archiv geliefert.".into());
    }
    let url = Url::parse(&package.link).map_err(|_| "Ungültige Java-Downloadadresse.")?;
    if !destination(&url, major, true) || !url.path().ends_with(&format!("/{}", package.name)) {
        return Err(
            "Der Java-Download stammt nicht aus dem offiziellen Adoptium-Release-Repository."
                .into(),
        );
    }
    Ok(())
}

fn client() -> Result<Client, String> {
    Client::builder()
        .user_agent(concat!("SyncMC/", env!("CARGO_PKG_VERSION")))
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|_| "Der Java-Downloadclient konnte nicht gestartet werden.".into())
}

async fn select_package(major: u32, os: &str, arch: &str) -> Result<(Package, String), String> {
    let client = client()?;
    for image in ["jre", "jdk"] {
        let mut response = client
            .get(format!(
                "https://api.adoptium.net/v3/assets/latest/{major}/hotspot"
            ))
            .query(&[
                ("architecture", arch),
                ("image_type", image),
                ("os", os),
                ("vendor", "eclipse"),
            ])
            .send()
            .await
            .map_err(|_| "Adoptium ist nicht erreichbar. Bitte die Verbindung prüfen.")?;
        if response.status().as_u16() == 404 {
            continue;
        }
        if !response.status().is_success() {
            return Err(format!(
                "Die Java-Versionsliste konnte nicht geladen werden (HTTP {}).",
                response.status().as_u16()
            ));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "Die Java-Versionsliste wurde unterbrochen.")?
        {
            if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
                return Err("Die Java-Versionsliste ist zu groß.".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let assets: Vec<Asset> = serde_json::from_slice(&bytes)
            .map_err(|_| "Adoptium hat ungültige Versionsdaten geliefert.")?;
        for asset in assets {
            let binary = &asset.binary;
            if asset.vendor != "eclipse"
                || asset.version.major != major
                || binary.os != os
                || binary.architecture != arch
                || binary.image_type != image
                || binary.jvm_impl != "hotspot"
                || binary.heap_size != "normal"
                || asset.release_name.to_ascii_lowercase().contains("-ea")
            {
                continue;
            }
            check_package(&binary.package, major, os, arch, image)?;
            return Ok((asset.binary.package, asset.release_name));
        }
    }
    Err(format!("Adoptium bietet Java {major} für {os}/{arch} derzeit nicht als passendes Archiv an. Bitte eine passende Java-Installation manuell auswählen."))
}

fn file_hash(path: &Path) -> Result<(String, u64), String> {
    let mut file =
        File::open(path).map_err(|e| format!("Java-Datei konnte nicht gelesen werden: {e}"))?;
    let mut hash = Sha256::new();
    let mut size = 0;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        size += count as u64;
        if size > MAX_EXPANDED {
            return Err("Eine Java-Datei überschreitet die zulässige Größe.".into());
        }
        hash.update(&buffer[..count]);
    }
    Ok((format!("{:x}", hash.finalize()), size))
}

fn archive_valid(path: &Path, package: &Package) -> Result<bool, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.to_string()),
    };
    if !metadata.is_file() || metadata.len() != package.size {
        return Ok(false);
    }
    Ok(file_hash(path)?.0.eq_ignore_ascii_case(&package.checksum))
}

async fn download(
    java_root: &Path,
    major: u32,
    package: &Package,
    progress: &Progress,
) -> Result<PathBuf, String> {
    let ext = if package.name.ends_with(".zip") {
        "zip"
    } else {
        "tar.gz"
    };
    let relative =
        PathBuf::from("archives").join(format!("{}.{}", package.checksum.to_lowercase(), ext));
    let target = managed_path(java_root, &relative, false)?;
    fs::create_dir_all(target.parent().ok_or("Java-Cachepfad fehlt.")?)
        .map_err(|e| e.to_string())?;
    if archive_valid(&target, package)? {
        return Ok(target);
    }
    let partial = relative.with_extension(format!("{}.part", uuid::Uuid::new_v4()));
    let temp = managed_path(java_root, &partial, false)?;
    let _cleanup = Cleanup(temp.clone());
    let client = Client::builder()
        .user_agent(concat!("SyncMC/", env!("CARGO_PKG_VERSION")))
        .https_only(true)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(300))
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= 5 || !destination(attempt.url(), major, false) {
                attempt.error("Unzulässige Java-Downloadweiterleitung")
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|_| "Java-Download konnte nicht vorbereitet werden.")?;
    let mut response = client
        .get(&package.link)
        .send()
        .await
        .map_err(|_| "Das Java-Archiv konnte nicht heruntergeladen werden.")?;
    if !response.status().is_success() {
        return Err(format!(
            "Der Java-Download ist fehlgeschlagen (HTTP {}).",
            response.status().as_u16()
        ));
    }
    if response
        .content_length()
        .is_some_and(|size| size != package.size)
    {
        return Err("Die angekündigte Java-Archivgröße stimmt nicht mit Adoptium überein.".into());
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut received = 0;
    let mut reported = 0;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Der Java-Download wurde unterbrochen. Bitte erneut versuchen.")?
    {
        received += chunk.len() as u64;
        if received > package.size {
            return Err("Das Java-Archiv ist größer als erwartet.".into());
        }
        file.write_all(&chunk)
            .map_err(|e| format!("Das Java-Archiv konnte nicht gespeichert werden: {e}"))?;
        hasher.update(&chunk);
        if received - reported >= 512 * 1024 || received == package.size {
            progress(JavaProgress {
                major,
                stage: "downloading",
                downloaded_bytes: received,
                total_bytes: package.size,
            });
            reported = received;
        }
    }
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    if received != package.size
        || !format!("{:x}", hasher.finalize()).eq_ignore_ascii_case(&package.checksum)
    {
        return Err("Die SHA-256-Prüfsumme oder Größe des Java-Archivs stimmt nicht. Es wurde nicht installiert.".into());
    }
    #[cfg(windows)]
    if target.exists() {
        fs::remove_file(&target).map_err(|e| e.to_string())?;
    }
    fs::rename(&temp, &target).map_err(|e| e.to_string())?;
    Ok(target)
}

struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.0).is_ok_and(|m| m.is_dir()) {
            let _ = fs::remove_dir_all(&self.0);
        } else {
            let _ = fs::remove_file(&self.0);
        }
    }
}

#[derive(Clone)]
struct ArchiveLink {
    path: PathBuf,
    target: PathBuf,
    resolved: PathBuf,
    hard: bool,
}

fn link_destination(path: &Path, target: &Path, hard: bool) -> Result<PathBuf, String> {
    let text = target
        .to_str()
        .ok_or("Ungültiger Verweis im Java-Archiv.")?;
    if text.is_empty()
        || text.len() > 2048
        || text.starts_with('/')
        || text.contains(['\\', ':', '\0'])
        || text.chars().any(char::is_control)
    {
        return Err("Unsicherer Verweis im Java-Archiv.".into());
    }
    let mut parts = if hard {
        Vec::new()
    } else {
        path.parent()
            .unwrap_or(Path::new(""))
            .components()
            .map(|p| p.as_os_str().to_os_string())
            .collect()
    };
    for part in target.components() {
        match part {
            Component::Normal(name) => {
                safe_component(name.to_str().ok_or("Ungültiger Verweis im Java-Archiv.")?)?;
                parts.push(name.to_os_string());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if parts.pop().is_none() {
                    return Err("Ein Java-Archivverweis verlässt den Installationsordner.".into());
                }
            }
            _ => return Err("Unsicherer Verweis im Java-Archiv.".into()),
        }
    }
    let result: PathBuf = parts.into_iter().collect();
    wire(&result)?;
    Ok(result)
}

fn final_link_target(
    path: &Path,
    links: &BTreeMap<PathBuf, ArchiveLink>,
) -> Result<PathBuf, String> {
    let mut current = path.to_path_buf();
    let mut visited = BTreeSet::new();
    for _ in 0..64 {
        if !visited.insert(current.clone()) {
            return Err("Zyklischer Verweis im Java-Archiv.".into());
        }
        let parts: Vec<_> = current
            .components()
            .map(|p| p.as_os_str().to_owned())
            .collect();
        let mut prefix = PathBuf::new();
        let mut replacement = None;
        for (index, part) in parts.iter().enumerate() {
            prefix.push(part);
            if let Some(link) = links.get(&prefix) {
                let mut next = link.resolved.clone();
                for suffix in &parts[index + 1..] {
                    next.push(suffix);
                }
                replacement = Some(next);
                break;
            }
        }
        match replacement {
            Some(next) => current = next,
            None => return Ok(current),
        }
    }
    Err("Zu viele verschachtelte Verweise im Java-Archiv.".into())
}

fn install_links(root: &Path, entries: Vec<ArchiveLink>) -> Result<(), String> {
    let links: BTreeMap<_, _> = entries
        .into_iter()
        .map(|link| (link.path.clone(), link))
        .collect();
    for link in links.values() {
        let destination = managed_path(root, &link.path, false)?;
        if destination.exists() {
            return Err("Kollidierender Verweis im Java-Archiv.".into());
        }
        let final_target = final_link_target(&link.resolved, &links)?;
        let final_path = managed_path(root, &final_target, false)?;
        let metadata = fs::symlink_metadata(&final_path)
            .map_err(|_| "Ein Verweis im Java-Archiv hat kein gültiges Ziel.")?;
        if !metadata.is_file() && !metadata.is_dir() {
            return Err("Ungültiges Verweisziel im Java-Archiv.".into());
        }
        fs::create_dir_all(destination.parent().ok_or("Java-Verweispfad fehlt.")?)
            .map_err(|e| e.to_string())?;
        if link.hard {
            if !metadata.is_file() {
                return Err("Ein Java-Hardlink verweist nicht auf eine Datei.".into());
            }
            fs::hard_link(final_path, destination)
                .map_err(|e| format!("Java-Hardlink konnte nicht erstellt werden: {e}"))?;
        } else {
            #[cfg(unix)]
            std::os::unix::fs::symlink(&link.target, destination)
                .map_err(|e| format!("Java-Verweis konnte nicht erstellt werden: {e}"))?;
            #[cfg(not(unix))]
            return Err("Das Java-Archiv enthält einen auf diesem System nicht unterstützten symbolischen Link.".into());
        }
    }
    Ok(())
}

fn write_entry(
    root: &Path,
    relative: &Path,
    mut reader: impl Read,
    size: u64,
    executable: bool,
) -> Result<(), String> {
    if size > MAX_ARCHIVE {
        return Err("Eine Datei im Java-Archiv ist zu groß.".into());
    }
    let path = managed_path(root, relative, false)?;
    fs::create_dir_all(path.parent().ok_or("Java-Dateipfad fehlt.")?).map_err(|e| e.to_string())?;
    let mut target = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| format!("Java-Datei konnte nicht entpackt werden: {e}"))?;
    let copied = std::io::copy(&mut reader.by_ref().take(size + 1), &mut target)
        .map_err(|e| format!("Java-Archiv konnte nicht entpackt werden: {e}"))?;
    if copied != size {
        return Err("Die entpackte Java-Dateigröße stimmt nicht.".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            &path,
            fs::Permissions::from_mode(if executable { 0o755 } else { 0o644 }),
        )
        .map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    let _ = executable;
    Ok(())
}

fn extract_archive(archive: &Path, destination: &Path, zipped: bool) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    let mut links = Vec::new();
    let mut expanded = 0u64;
    let mut register = |path: &Path, size: u64| -> Result<(), String> {
        let path = wire(path)?;
        if path == PROOF || !seen.insert(path.to_lowercase()) || seen.len() > MAX_FILES {
            return Err("Kollidierende oder zu viele Dateien im Java-Archiv.".into());
        }
        expanded = expanded
            .checked_add(size)
            .ok_or("Java-Archiv ist zu groß.")?;
        if expanded > MAX_EXPANDED {
            return Err("Das entpackte Java-Archiv ist zu groß.".into());
        }
        Ok(())
    };
    if zipped {
        let mut zip = zip::ZipArchive::new(File::open(archive).map_err(|e| e.to_string())?)
            .map_err(|_| "Das Java-ZIP-Archiv ist ungültig.")?;
        for index in 0..zip.len() {
            let mut entry = zip
                .by_index(index)
                .map_err(|_| "Ein Eintrag im Java-ZIP-Archiv ist beschädigt.")?;
            let path = relative(entry.name().trim_end_matches('/'))?;
            if entry.is_dir() {
                register(&path, 0)?;
                fs::create_dir_all(managed_path(destination, &path, false)?)
                    .map_err(|e| e.to_string())?;
                continue;
            }
            let mode = entry.unix_mode().unwrap_or(0o100644);
            let size = entry.size();
            register(&path, size)?;
            if mode & 0o170000 == 0o120000 {
                if size > 2048 {
                    return Err("Zu langer Verweis im Java-Archiv.".into());
                }
                let mut target = String::new();
                entry
                    .by_ref()
                    .take(2_049)
                    .read_to_string(&mut target)
                    .map_err(|_| "Ungültiger Java-Archivverweis.")?;
                if target.len() > 2_048 {
                    return Err("Zu langer Verweis im Java-Archiv.".into());
                }
                let target = PathBuf::from(target);
                links.push(ArchiveLink {
                    resolved: link_destination(&path, &target, false)?,
                    path,
                    target,
                    hard: false,
                });
            } else if mode & 0o170000 == 0 || mode & 0o170000 == 0o100000 {
                write_entry(destination, &path, &mut entry, size, mode & 0o111 != 0)?;
            } else {
                return Err("Nicht unterstützter Dateityp im Java-ZIP-Archiv.".into());
            }
        }
    } else {
        let decoder = GzDecoder::new(File::open(archive).map_err(|e| e.to_string())?);
        // Bound all decompressed bytes, including ignored header padding and directories.
        let mut tar = tar::Archive::new(decoder.take(MAX_EXPANDED + 64 * 1024 * 1024));
        for item in tar
            .entries()
            .map_err(|_| "Das Java-TAR-Archiv ist ungültig.")?
        {
            let mut entry = item.map_err(|_| "Ein Eintrag im Java-TAR-Archiv ist beschädigt.")?;
            let raw = entry
                .path()
                .map_err(|_| "Ungültiger Pfad im Java-TAR-Archiv.")?
                .into_owned();
            let path = relative(raw.to_str().ok_or("Ungültiger Java-Archivpfad.")?)?;
            let kind = entry.header().entry_type();
            if kind.is_dir() {
                register(&path, 0)?;
                fs::create_dir_all(managed_path(destination, &path, false)?)
                    .map_err(|e| e.to_string())?;
                continue;
            }
            let size = entry.size();
            register(&path, size)?;
            if kind.is_file() {
                let mode = entry
                    .header()
                    .mode()
                    .map_err(|_| "Ungültige Java-Dateirechte.")?;
                write_entry(destination, &path, &mut entry, size, mode & 0o111 != 0)?;
            } else if kind.is_symlink() || kind.is_hard_link() {
                let target = entry
                    .link_name()
                    .map_err(|_| "Ungültiger Java-Archivverweis.")?
                    .ok_or("Verweisziel fehlt im Java-Archiv.")?
                    .into_owned();
                links.push(ArchiveLink {
                    resolved: link_destination(&path, &target, kind.is_hard_link())?,
                    path,
                    target,
                    hard: kind.is_hard_link(),
                });
            } else {
                return Err("Nicht unterstützter Dateityp im Java-TAR-Archiv.".into());
            }
        }
    }
    install_links(destination, links)
}

fn inventory(root: &Path) -> Result<(Vec<FileProof>, Vec<LinkProof>), String> {
    let canonical = fs::canonicalize(root).map_err(|e| e.to_string())?;
    let mut pending = vec![PathBuf::new()];
    let mut files = Vec::new();
    let mut links = Vec::new();
    let mut total = 0;
    while let Some(directory) = pending.pop() {
        let dir = if directory.as_os_str().is_empty() {
            root.to_path_buf()
        } else {
            managed_path(root, &directory, false)?
        };
        for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let relative = directory.join(entry.file_name());
            let path = wire(&relative)?;
            if path == PROOF {
                continue;
            }
            let metadata = fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
            if metadata.file_type().is_symlink() {
                let target = fs::read_link(entry.path()).map_err(|e| e.to_string())?;
                link_destination(&relative, &target, false)?;
                if !fs::canonicalize(entry.path())
                    .map_err(|_| "Ein Java-Verweis ist beschädigt.")?
                    .starts_with(&canonical)
                {
                    return Err("Ein Java-Verweis verlässt den Installationsordner.".into());
                }
                links.push(LinkProof {
                    path,
                    target: target.to_str().ok_or("Ungültiger Java-Verweis.")?.into(),
                });
            } else if metadata.is_dir() {
                pending.push(relative);
            } else if metadata.is_file() {
                let (sha256, size) = file_hash(&entry.path())?;
                total += size;
                if total > MAX_EXPANDED {
                    return Err("Die Java-Installation ist zu groß.".into());
                }
                #[cfg(unix)]
                let executable = {
                    use std::os::unix::fs::PermissionsExt;
                    metadata.permissions().mode() & 0o111 != 0
                };
                #[cfg(not(unix))]
                let executable = false;
                files.push(FileProof {
                    path,
                    sha256,
                    size,
                    executable,
                });
            } else {
                return Err("Nicht unterstützte Datei in der Java-Installation.".into());
            }
            if files.len() + links.len() + pending.len() > MAX_FILES {
                return Err("Zu viele Dateien in der Java-Installation.".into());
            }
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    links.sort_by(|a, b| a.path.cmp(&b.path));
    Ok((files, links))
}

fn executable(files: &[FileProof], os: &str) -> Result<String, String> {
    let candidates: Vec<_> = files
        .iter()
        .filter(|file| {
            let parts: Vec<_> = file.path.split('/').collect();
            if os == "mac" {
                parts.len() == 5 && parts[1..] == ["Contents", "Home", "bin", "java"]
            } else {
                parts.len() == 3
                    && parts[1] == "bin"
                    && parts[2] == if os == "windows" { "java.exe" } else { "java" }
            }
        })
        .collect();
    if candidates.len() != 1 {
        return Err(
            "Im Java-Archiv wurde kein eindeutiger passender Java-Startpfad gefunden.".into(),
        );
    }
    Ok(candidates[0].path.clone())
}

/// Verifies every installed byte and every symlink before returning an executable path.
pub fn installed(app_data: &Path, major: u32) -> Result<Option<PathBuf>, String> {
    // Unsupported majors can still be supplied by an existing manually installed JVM.
    if supported_major(major).is_err() {
        return Ok(None);
    }
    let (os, arch) = host()?;
    let java_root = root(app_data)?;
    let directory = managed_path(&java_root, &slot(major, os, arch), false)?;
    if !directory.exists() {
        return Ok(None);
    }
    let proof_path = managed_path(&directory, Path::new(PROOF), false)?;
    let verify = || -> Result<PathBuf, String> {
        if fs::metadata(&proof_path).map_err(|e| e.to_string())?.len() > 16 * 1024 * 1024 {
            return Err("Ungültiger Java-Installationsnachweis.".into());
        }
        let proof: RuntimeProof =
            serde_json::from_slice(&fs::read(proof_path).map_err(|e| e.to_string())?)
                .map_err(|_| "Ungültiger Java-Installationsnachweis.")?;
        if proof.format != "syncmc-java-runtime"
            || proof.version != 1
            || proof.major != major
            || proof.os != os
            || proof.arch != arch
        {
            return Err("Der Java-Installationsnachweis passt nicht zu diesem Gerät.".into());
        }
        let image = if proof.package.name.contains("-jre_") {
            "jre"
        } else {
            "jdk"
        };
        check_package(&proof.package, major, os, arch, image)?;
        let (files, links) = inventory(&directory)?;
        if files.is_empty()
            || files != proof.files
            || links != proof.links
            || executable(&files, os)? != proof.executable
        {
            return Err(
                "Die Java-Dateien stimmen nicht mit dem Installationsnachweis überein.".into(),
            );
        }
        managed_path(&directory, &relative(&proof.executable)?, false)
    };
    verify().map(Some).map_err(|_| format!("Die verwaltete Java-{major}-Installation ist beschädigt. Bitte „Java automatisch einrichten“ erneut ausführen, um sie zu reparieren."))
}

/// Installs only into this app's data directory. Callers explicitly request this operation.
pub async fn install_at(
    app_data: &Path,
    major: u32,
    progress: Progress,
) -> Result<RuntimeInfo, String> {
    supported_major(major)?;
    let _guard = INSTALL_LOCK
        .try_lock()
        .map_err(|_| "Eine Java-Einrichtung läuft bereits. Bitte warten.")?;
    let (os, arch) = host()?;
    if let Ok(Some(path)) = installed(app_data, major) {
        let info = runtime::validate(&path.to_string_lossy(), major).await?;
        progress(JavaProgress {
            major,
            stage: "complete",
            downloaded_bytes: 0,
            total_bytes: 0,
        });
        return Ok(info);
    }
    progress(JavaProgress {
        major,
        stage: "metadata",
        downloaded_bytes: 0,
        total_bytes: 0,
    });
    let (package, release) = select_package(major, os, arch).await?;
    let java_root = root(app_data)?;
    fs::create_dir_all(&java_root)
        .map_err(|e| format!("Der Java-Datenordner konnte nicht erstellt werden: {e}"))?;
    let archive = download(&java_root, major, &package, &progress).await?;
    let stage_relative = PathBuf::from(format!(".stage-{}", uuid::Uuid::new_v4()));
    let staging = managed_path(&java_root, &stage_relative, false)?;
    fs::create_dir(&staging).map_err(|e| e.to_string())?;
    let _cleanup = Cleanup(staging.clone());
    progress(JavaProgress {
        major,
        stage: "extracting",
        downloaded_bytes: package.size,
        total_bytes: package.size,
    });
    let extract_path = staging.clone();
    let zipped = package.name.ends_with(".zip");
    tokio::task::spawn_blocking(move || extract_archive(&archive, &extract_path, zipped))
        .await
        .map_err(|_| "Das Java-Archiv konnte nicht entpackt werden.")??;
    progress(JavaProgress {
        major,
        stage: "verifying",
        downloaded_bytes: package.size,
        total_bytes: package.size,
    });
    let (files, links) = inventory(&staging)?;
    let executable = executable(&files, os)?;
    let java = managed_path(&staging, &relative(&executable)?, false)?;
    runtime::validate(&java.to_string_lossy(), major).await?;
    let proof = RuntimeProof {
        format: "syncmc-java-runtime".into(),
        version: 1,
        major,
        os: os.into(),
        arch: arch.into(),
        release,
        package: package.clone(),
        executable,
        files,
        links,
    };
    let proof_path = managed_path(&staging, Path::new(PROOF), false)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(proof_path)
        .map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec_pretty(&proof).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    let target = managed_path(&java_root, &slot(major, os, arch), false)?;
    let backup = managed_path(
        &java_root,
        Path::new(&format!(".old-{}", uuid::Uuid::new_v4())),
        false,
    )?;
    let existed = target.exists();
    if existed {
        if !target.is_dir() {
            return Err("Am verwalteten Java-Installationspfad liegt eine fremde Datei.".into());
        }
        fs::rename(&target, &backup).map_err(|e| {
            format!(
                "Die alte Java-Installation konnte nicht für die Reparatur gesichert werden: {e}"
            )
        })?;
    }
    if let Err(error) = fs::rename(&staging, &target) {
        if existed {
            let _ = fs::rename(&backup, &target);
        }
        return Err(format!(
            "Die geprüfte Java-Installation konnte nicht übernommen werden: {error}"
        ));
    }
    if existed {
        let _ = fs::remove_dir_all(backup);
    }
    let path = installed(app_data, major)?
        .ok_or("Der Java-Installationsabschluss konnte nicht geprüft werden.")?;
    let info = runtime::validate(&path.to_string_lossy(), major).await?;
    progress(JavaProgress {
        major,
        stage: "complete",
        downloaded_bytes: package.size,
        total_bytes: package.size,
    });
    Ok(info)
}

#[tauri::command]
pub async fn install_java_runtime(
    app: tauri::AppHandle,
    required_major: u32,
) -> Result<RuntimeInfo, String> {
    let _guard = crate::minecraft::INSTALL_LOCK
        .try_lock()
        .map_err(|_| "Eine Installation oder ein Spielstart läuft bereits.")?;
    crate::launch::ensure_stopped()?;
    let data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let progress: Progress = Arc::new(move |event| {
        let _ = app.emit("java-install-progress", event);
    });
    install_at(&data, required_major, progress).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{write::GzEncoder, Compression};
    use std::io::Cursor;

    fn temporary() -> Cleanup {
        let path = std::env::temp_dir().join(format!("syncmc-java-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Cleanup(path)
    }

    fn package(major: u32, os: &str, arch: &str) -> Package {
        let name = format!(
            "OpenJDK{major}U-jre_{arch}_{os}_hotspot_test{}",
            if os == "windows" { ".zip" } else { ".tar.gz" }
        );
        Package {
            checksum: "a".repeat(64),
            size: 10,
            link: format!(
                "https://github.com/adoptium/temurin{major}-binaries/releases/download/test/{name}"
            ),
            name,
        }
    }

    fn tar_fixture(path: &Path, entries: &[(&str, tar::EntryType, &str, &[u8])]) {
        let encoder = GzEncoder::new(File::create(path).unwrap(), Compression::fast());
        let mut builder = tar::Builder::new(encoder);
        for (name, kind, target, contents) in entries {
            let mut header = tar::Header::new_gnu();
            // Raw names deliberately permit fixtures which safe archive builders reject.
            header.as_mut_bytes()[..100].fill(0);
            header.as_mut_bytes()[..name.len()].copy_from_slice(name.as_bytes());
            header.set_size(contents.len() as u64);
            header.set_mode(0o755);
            header.set_entry_type(*kind);
            if !target.is_empty() {
                header.set_link_name(target).unwrap();
            }
            header.set_cksum();
            builder.append(&header, Cursor::new(*contents)).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap();
    }

    fn proof_fixture(app_data: &Path) -> (PathBuf, PathBuf) {
        let (os, arch) = host().unwrap();
        let java_root = root(app_data).unwrap();
        let directory = java_root.join(slot(21, os, arch));
        let binary = directory.join(if os == "mac" {
            "jdk-test/Contents/Home/bin/java"
        } else if os == "windows" {
            "jdk-test/bin/java.exe"
        } else {
            "jdk-test/bin/java"
        });
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::write(&binary, b"fixture Java bytes, never executed").unwrap();
        let (files, links) = inventory(&directory).unwrap();
        let executable = executable(&files, os).unwrap();
        let proof = RuntimeProof {
            format: "syncmc-java-runtime".into(),
            version: 1,
            major: 21,
            os: os.into(),
            arch: arch.into(),
            release: "test".into(),
            package: package(21, os, arch),
            executable,
            files,
            links,
        };
        fs::write(directory.join(PROOF), serde_json::to_vec(&proof).unwrap()).unwrap();
        (directory, binary)
    }

    #[test]
    fn accepts_only_matching_temurin_packages_and_official_release_redirects() {
        let mut item = package(21, "mac", "aarch64");
        assert!(check_package(&item, 21, "mac", "aarch64", "jre").is_ok());
        assert!(check_package(&item, 21, "windows", "x64", "jre").is_err());
        item.link = item
            .link
            .replace("github.com/adoptium/", "github.com/untrusted/");
        assert!(check_package(&item, 21, "mac", "aarch64", "jre").is_err());
        item = package(21, "mac", "aarch64");
        item.checksum = "invalid".into();
        assert!(check_package(&item, 21, "mac", "aarch64", "jre").is_err());
        assert!(destination(
            &Url::parse("https://release-assets.githubusercontent.com/a/release?signature=public")
                .unwrap(),
            21,
            false
        ));
        for bad in [
            "http://release-assets.githubusercontent.com/a",
            "https://github.com.evil.test/adoptium/temurin21-binaries/releases/download/a/b",
            "https://example.org/a",
            "https://user:pass@github.com/adoptium/temurin21-binaries/releases/download/a/b",
        ] {
            assert!(!destination(&Url::parse(bad).unwrap(), 21, false));
        }
        assert!(supported_major(9).is_err());
    }

    #[test]
    fn tar_rejects_traversal_absolute_paths_and_escaping_links() {
        for (name, kind, target) in [
            ("../outside", tar::EntryType::Regular, ""),
            ("/outside", tar::EntryType::Regular, ""),
            ("jdk/bin/link", tar::EntryType::Symlink, "../../../outside"),
        ] {
            let data = temporary();
            let archive = data.0.join("runtime.tar.gz");
            let destination = data.0.join("extract");
            fs::create_dir(&destination).unwrap();
            tar_fixture(&archive, &[(name, kind, target, b"")]);
            assert!(
                extract_archive(&archive, &destination, false).is_err(),
                "{name}"
            );
            assert!(!data.0.join("outside").exists());
        }
    }

    #[test]
    fn zip_rejects_traversal_and_duplicate_case_insensitive_paths() {
        use zip::write::SimpleFileOptions;
        for names in [vec!["../outside"], vec!["jdk/bin/java", "jdk/bin/JAVA"]] {
            let data = temporary();
            let archive = data.0.join("runtime.zip");
            let destination = data.0.join("extract");
            fs::create_dir(&destination).unwrap();
            let mut writer = zip::ZipWriter::new(File::create(&archive).unwrap());
            for name in names {
                writer
                    .start_file(name, SimpleFileOptions::default())
                    .unwrap();
                writer.write_all(b"test").unwrap();
            }
            writer.finish().unwrap();
            assert!(extract_archive(&archive, &destination, true).is_err());
            assert!(!data.0.join("outside").exists());
        }
    }

    #[cfg(unix)]
    #[test]
    fn preserves_internal_archive_links_but_rejects_symlink_write_pivots() {
        let data = temporary();
        let archive = data.0.join("runtime.tar.gz");
        let destination = data.0.join("extract");
        fs::create_dir(&destination).unwrap();
        tar_fixture(
            &archive,
            &[
                ("jdk/bin/java", tar::EntryType::Regular, "", b"java"),
                ("jdk/bin/java-link", tar::EntryType::Symlink, "java", b""),
            ],
        );
        extract_archive(&archive, &destination, false).unwrap();
        assert_eq!(
            fs::read(destination.join("jdk/bin/java-link")).unwrap(),
            b"java"
        );
        let (_, links) = inventory(&destination).unwrap();
        assert_eq!(links.len(), 1);
        let pivot_destination = data.0.join("pivot-extract");
        fs::create_dir(&pivot_destination).unwrap();
        let pivot = data.0.join("pivot.tar.gz");
        tar_fixture(
            &pivot,
            &[
                ("jdk/pivot", tar::EntryType::Symlink, "../../outside", b""),
                (
                    "jdk/pivot/file",
                    tar::EntryType::Regular,
                    "",
                    b"must not escape",
                ),
            ],
        );
        assert!(extract_archive(&pivot, &pivot_destination, false).is_err());
        assert!(!data.0.join("outside").exists());
    }

    #[test]
    fn local_proof_detects_corrupted_missing_and_untracked_runtime_files() {
        let data = temporary();
        let (directory, binary) = proof_fixture(&data.0);
        assert_eq!(installed(&data.0, 21).unwrap().unwrap(), binary);
        fs::write(&binary, b"corrupted").unwrap();
        assert!(installed(&data.0, 21).unwrap_err().contains("reparieren"));
        fs::write(&binary, b"fixture Java bytes, never executed").unwrap();
        assert!(installed(&data.0, 21).unwrap().is_some());
        fs::write(directory.join("foreign-agent.jar"), b"unexpected").unwrap();
        assert!(installed(&data.0, 21).is_err());
        fs::remove_file(directory.join("foreign-agent.jar")).unwrap();
        fs::remove_file(&binary).unwrap();
        assert!(installed(&data.0, 21).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn existing_symlinks_cannot_redirect_runtime_writes_or_verification() {
        let data = temporary();
        let outside = temporary();
        std::os::unix::fs::symlink(&outside.0, data.0.join("java-runtimes")).unwrap();
        assert!(root(&data.0).is_err());
        assert!(fs::read_dir(&outside.0).unwrap().next().is_none());
    }

    #[test]
    fn damaged_archive_never_counts_as_verified_cache() {
        let data = temporary();
        let path = data.0.join("archive");
        let mut item = package(21, "mac", "aarch64");
        item.size = 4;
        item.checksum = format!("{:x}", Sha256::digest(b"good"));
        assert!(!archive_valid(&path, &item).unwrap());
        fs::write(&path, b"bad!").unwrap();
        assert!(!archive_valid(&path, &item).unwrap());
        fs::write(&path, b"good").unwrap();
        assert!(archive_valid(&path, &item).unwrap());
        fs::write(&path, b"good but truncated metadata").unwrap();
        assert!(!archive_valid(&path, &item).unwrap());
    }

    #[test]
    #[ignore = "Downloads a real Temurin runtime into /private/tmp; validates reuse and corruption repair, never starts Minecraft."]
    fn real_managed_java_installation() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let data = std::env::var_os("SYNCMC_JAVA_TEST_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/private/tmp/syncmc-managed-java-smoke"));
            let major = std::env::var("SYNCMC_JAVA_TEST_MAJOR")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(21);
            let progress: Progress =
                Arc::new(|p| eprintln!("JAVA_PROGRESS {}", serde_json::to_string(&p).unwrap()));
            let first = install_at(&data, major, progress.clone()).await.unwrap();
            assert_eq!(first.java_major, Some(major));
            assert!(Path::new(&first.java_path).starts_with(data.join("java-runtimes")));
            let reused = install_at(
                &data,
                major,
                Arc::new(|p| assert_ne!(p.stage, "downloading")),
            )
            .await
            .unwrap();
            assert_eq!(first.java_path, reused.java_path);
            fs::write(
                &first.java_path,
                b"deliberately corrupted smoke-test executable",
            )
            .unwrap();
            assert!(installed(&data, major).is_err());
            let repaired = install_at(&data, major, progress).await.unwrap();
            assert_eq!(repaired.java_major, Some(major));
            assert!(installed(&data, major).unwrap().is_some());
            eprintln!("JAVA_RECEIPT {}", serde_json::to_string(&repaired).unwrap());
        });
    }
}

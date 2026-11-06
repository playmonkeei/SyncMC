//! Local, verified instance snapshots. Worlds and user configuration are included; shared
//! Minecraft libraries/assets and account credentials live outside the instance directory.
use crate::content::{self, ContentSelection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha512};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::Manager;

const MAX_BYTES: u64 = 20 * 1024 * 1024 * 1024;
const MAX_FILES: usize = 200_000;
const MAX_DIRECTORIES: usize = 100_000;
const MAX_MANIFEST_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupSummary {
    pub id: String,
    pub instance_id: String,
    pub created_at: u64,
    pub reason: String,
    pub file_count: usize,
    pub total_bytes: u64,
    pub mc_version: String,
    pub loader: String,
    pub profile_name: String,
    pub contents_count: usize,
    pub verified_content: bool,
    pub contained_sections: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupRestoreReport {
    pub backup: BackupSummary,
    pub rescue_backup: BackupSummary,
    pub profile: Value,
    pub contents: Vec<ContentSelection>,
    pub requires_installation: bool,
    pub requires_content_install: bool,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Profile {
    id: String,
    name: String,
    mc_version: String,
    loader: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    loader_version: Option<String>,
    cover: String,
    contents: Vec<ContentSelection>,
    servers: Vec<Server>,
}

#[derive(Clone, Deserialize, Serialize)]
struct Server {
    id: String,
    name: String,
    address: String,
}

fn text(value: &str, maximum: usize, label: &str) -> Result<(), String> {
    if value.trim() != value
        || value.is_empty()
        || value.chars().count() > maximum
        || value.chars().any(char::is_control)
    {
        return Err(format!(
            "{label} ist leer, zu lang oder enthält ungültige Zeichen."
        ));
    }
    Ok(())
}

fn identifier(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 100
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        return Err("Ungültige Instanz- oder Sicherungs-ID.".into());
    }
    Ok(())
}

/// Whitelist shared profile fields; device settings and credentials can never enter a snapshot.
pub(crate) fn normalize_profile(value: Value, instance_id: &str) -> Result<Value, String> {
    if serde_json::to_vec(&value).map_err(|e| e.to_string())?.len() > 5_000_000 {
        return Err("Das Instanzprofil ist zu groß.".into());
    }
    let profile: Profile = serde_json::from_value(value)
        .map_err(|_| "Das Instanzprofil für die Sicherung ist unvollständig.")?;
    identifier(instance_id)?;
    if profile.id != instance_id {
        return Err("Das Sicherungsprofil gehört zu einer anderen Instanz.".into());
    }
    text(&profile.name, 100, "Instanzname")?;
    text(&profile.mc_version, 50, "Minecraft-Version")?;
    if !profile
        .mc_version
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b" ._+-".contains(&b))
        || !matches!(
            profile.loader.as_str(),
            "vanilla" | "fabric" | "forge" | "neoforge"
        )
        || !matches!(profile.cover.as_str(), "redwood" | "vanilla" | "create")
    {
        return Err(
            "Das Sicherungsprofil enthält eine ungültige Version, einen Loader oder ein Cover."
                .into(),
        );
    }
    if let Some(loader) = &profile.loader_version {
        if loader.is_empty()
            || loader.len() > 100
            || !loader
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
        {
            return Err("Ungültige Modloader-Version im Sicherungsprofil.".into());
        }
    }
    content::validate_selections(&profile.contents)?;
    if profile.servers.len() > 200 {
        return Err("Das Sicherungsprofil enthält zu viele Server.".into());
    }
    let mut ids = BTreeSet::new();
    let address = regex::Regex::new(
        r"^(?:\[[a-fA-F0-9:]+\]|[a-zA-Z0-9](?:[a-zA-Z0-9.-]*[a-zA-Z0-9])?)(?::(\d{1,5}))?$",
    )
    .unwrap();
    for server in &profile.servers {
        identifier(&server.id)?;
        text(&server.name, 160, "Servername")?;
        text(&server.address, 253, "Serveradresse")?;
        let captures = address
            .captures(&server.address)
            .ok_or("Ungültige Serveradresse im Sicherungsprofil.")?;
        if captures.get(1).is_some_and(|port| {
            port.as_str()
                .parse::<u32>()
                .map_or(true, |p| !(1..=65535).contains(&p))
        }) || !ids.insert(&server.id)
        {
            return Err(
                "Ungültiger Serverport oder doppelte Server-ID im Sicherungsprofil.".into(),
            );
        }
    }
    serde_json::to_value(profile).map_err(|e| e.to_string())
}

pub(crate) fn default_profile(
    instance: &str,
    game: &str,
    loader: &str,
    contents: &[ContentSelection],
) -> Value {
    json!({"id":instance,"name":instance,"mcVersion":game,"loader":loader,"cover":"vanilla","contents":contents,"servers":[]})
}

fn safe_relative(relative: &Path) -> Result<(), String> {
    if relative.as_os_str().is_empty() {
        return Err("Leerer Sicherungspfad.".into());
    }
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err("Unsicherer Pfad in einer Sicherung.".into());
        };
        let name = name
            .to_str()
            .ok_or("Ein Dateiname ist kein gültiges UTF-8 und kann nicht gesichert werden.")?;
        if name.is_empty()
            || name.len() > 255
            || name
                .chars()
                .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
            || name.ends_with(['.', ' '])
        {
            return Err("Ein Dateiname der Sicherung ist ungültig.".into());
        }
    }
    Ok(())
}

fn path(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    safe_relative(relative)?;
    let mut current = root.to_path_buf();
    if fs::symlink_metadata(root).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("Der Datenordner darf kein symbolischer Link sein.".into());
    }
    for component in relative.components() {
        current.push(component);
        if fs::symlink_metadata(&current).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err("Sicherungen folgen keinen symbolischen Links. Bitte den verknüpften Instanzpfad zuerst auflösen.".into());
        }
    }
    Ok(current)
}

fn posix(relative: &Path) -> Result<String, String> {
    safe_relative(relative)?;
    relative
        .components()
        .map(|part| match part {
            Component::Normal(name) => name
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| "Ungültiger Dateiname.".into()),
            _ => Err("Ungültiger Dateipfad.".into()),
        })
        .collect::<Result<Vec<_>, String>>()
        .map(|parts| parts.join("/"))
}

fn from_posix(value: &str) -> Result<PathBuf, String> {
    if value.is_empty()
        || value.contains('\\')
        || value
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err("Unsicherer Dateipfad im Sicherungsmanifest.".into());
    }
    let relative: PathBuf = value.split('/').collect();
    safe_relative(&relative)?;
    Ok(relative)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct BackupFile {
    path: String,
    sha512: String,
    size: u64,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    format: String,
    version: u8,
    summary: BackupSummary,
    profile: Value,
    directories: Vec<String>,
    files: Vec<BackupFile>,
}

fn backup_base(instance: &str, backup: &str) -> Result<PathBuf, String> {
    identifier(instance)?;
    identifier(backup)?;
    Ok(PathBuf::from("backups").join(instance).join(backup))
}

fn write_json(root: &Path, relative: &Path, value: &impl Serialize) -> Result<(), String> {
    let target = path(root, relative)?;
    fs::create_dir_all(target.parent().ok_or("Sicherungspfad fehlt.")?)
        .map_err(|e| e.to_string())?;
    let temp = path(root, &relative.with_extension("syncmc-part"))?;
    let mut output = File::create(&temp).map_err(|e| e.to_string())?;
    output
        .write_all(&serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    output.sync_all().map_err(|e| e.to_string())?;
    drop(output);
    #[cfg(windows)]
    if target.exists() {
        fs::remove_file(&target).map_err(|e| e.to_string())?;
    }
    fs::rename(temp, target).map_err(|e| e.to_string())
}

fn read_manifest(root: &Path, instance: &str, backup: &str) -> Result<Manifest, String> {
    let target = path(root, &backup_base(instance, backup)?.join("manifest.json"))?;
    let file = File::open(target).map_err(|_| "Die Sicherung ist nicht verfügbar.")?;
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_MANIFEST_BYTES {
        return Err("Das Sicherungsmanifest ist zu groß.".into());
    }
    let mut manifest: Manifest =
        serde_json::from_reader(file).map_err(|_| "Das Sicherungsmanifest ist beschädigt.")?;
    if manifest.format != "syncmc-instance-backup"
        || manifest.version != 1
        || manifest.summary.id != backup
        || manifest.summary.instance_id != instance
        || manifest.files.len() > MAX_FILES
        || manifest.directories.len() > MAX_DIRECTORIES
        || manifest.summary.total_bytes > MAX_BYTES
    {
        return Err("Ungültiges Sicherungsmanifest.".into());
    }
    manifest.profile = normalize_profile(manifest.profile, instance)?;
    let mut files = BTreeSet::new();
    let mut directories = BTreeSet::new();
    let mut bytes = 0_u64;
    for directory in &manifest.directories {
        from_posix(directory)?;
        if !directories.insert(directory.to_lowercase()) {
            return Err("Doppelte Verzeichnisse in der Sicherung.".into());
        }
    }
    for file in &manifest.files {
        from_posix(&file.path)?;
        if file.sha512.len() != 128
            || !file.sha512.bytes().all(|b| b.is_ascii_hexdigit())
            || !files.insert(file.path.to_lowercase())
            || directories.contains(&file.path.to_lowercase())
        {
            return Err("Ungültige oder doppelte Datei im Sicherungsmanifest.".into());
        }
        bytes = bytes
            .checked_add(file.size)
            .filter(|total| *total <= MAX_BYTES)
            .ok_or("Die Sicherung überschreitet die Größenbegrenzung.")?;
    }
    if bytes != manifest.summary.total_bytes || manifest.files.len() != manifest.summary.file_count
    {
        return Err("Die Sicherung ist unvollständig.".into());
    }
    if manifest.profile["mcVersion"] != manifest.summary.mc_version
        || manifest.profile["loader"] != manifest.summary.loader
    {
        return Err("Das gesicherte Profil passt nicht zum Sicherungsmanifest.".into());
    }
    Ok(manifest)
}

fn hash_file(file: &Path) -> Result<(u64, String), String> {
    let mut input = File::open(file)
        .map_err(|e| format!("Sicherungsdatei konnte nicht gelesen werden: {e}"))?;
    let mut hash = Sha512::new();
    let mut bytes = 0_u64;
    let mut buffer = [0; 128 * 1024];
    loop {
        let count = input.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        if bytes > MAX_BYTES {
            return Err("Eine Sicherungsdatei ist zu groß.".into());
        }
        hash.update(&buffer[..count]);
    }
    Ok((bytes, hex::encode(hash.finalize())))
}

fn copy_verified(
    source: &Path,
    destination: &Path,
    expected: Option<&BackupFile>,
) -> Result<(u64, String), String> {
    if fs::symlink_metadata(source)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("Eine Sicherungsdatei wurde durch einen symbolischen Link ersetzt.".into());
    }
    fs::create_dir_all(destination.parent().ok_or("Zielordner fehlt.")?)
        .map_err(|e| e.to_string())?;
    let mut input = File::open(source).map_err(|e| e.to_string())?;
    let initial = input.metadata().map_err(|e| e.to_string())?;
    if !initial.is_file() || initial.len() > MAX_BYTES {
        return Err("Eine Instanzdatei ist keine reguläre Datei oder überschreitet 20 GiB.".into());
    }
    let mut output = File::create(destination)
        .map_err(|e| format!("Die Sicherung konnte nicht geschrieben werden: {e}"))?;
    let mut bytes = 0_u64;
    let mut hash = Sha512::new();
    let mut buffer = [0; 128 * 1024];
    loop {
        let count = input.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        if bytes > initial.len() || bytes > MAX_BYTES {
            return Err("Eine Instanzdatei wurde während der Sicherung verändert.".into());
        }
        output
            .write_all(&buffer[..count])
            .map_err(|e| e.to_string())?;
        hash.update(&buffer[..count]);
    }
    output.sync_all().map_err(|e| e.to_string())?;
    drop(output);
    let hash = hex::encode(hash.finalize());
    let after = input.metadata().map_err(|e| e.to_string())?;
    if bytes != initial.len()
        || after.len() != initial.len()
        || initial.modified().ok() != after.modified().ok()
        || expected
            .is_some_and(|file| file.size != bytes || !file.sha512.eq_ignore_ascii_case(&hash))
    {
        return Err("Die Quelldatei wurde verändert oder ihre Prüfsumme stimmt nicht mit der Sicherung überein.".into());
    }
    let verified = hash_file(destination)?;
    if verified.0 != bytes || verified.1 != hash {
        return Err("Die geschriebene Sicherungsdatei ist beschädigt.".into());
    }
    Ok((bytes, hash))
}

#[derive(Default)]
struct Inventory {
    directories: Vec<PathBuf>,
    files: Vec<(PathBuf, u64)>,
    bytes: u64,
}

fn inventory(root: &Path, base: &Path) -> Result<Inventory, String> {
    fn walk(
        root: &Path,
        base: &Path,
        relative: &Path,
        depth: usize,
        result: &mut Inventory,
    ) -> Result<(), String> {
        if depth > 64 {
            return Err("Der Instanzordner ist zu tief verschachtelt.".into());
        }
        let target = if relative.as_os_str().is_empty() {
            path(root, base)?
        } else {
            path(root, &base.join(relative))?
        };
        let mut entries = fs::read_dir(target)
            .map_err(|e| format!("Der Instanzordner konnte nicht gesichert werden: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let child = relative.join(entry.file_name());
            safe_relative(&child)?;
            let source = path(root, &base.join(&child))?;
            let metadata = fs::symlink_metadata(source).map_err(|e| e.to_string())?;
            if metadata.is_dir() {
                result.directories.push(child.clone());
                if result.directories.len() > MAX_DIRECTORIES {
                    return Err("Die Instanz enthält zu viele Verzeichnisse für eine vollständige Sicherung.".into());
                }
                walk(root, base, &child, depth + 1, result)?;
            } else if metadata.is_file() {
                result.bytes=result.bytes.checked_add(metadata.len()).filter(|size|*size<=MAX_BYTES).ok_or("Die Instanz überschreitet die Sicherungsgrenze von 20 GiB. Es wurde keine unvollständige Sicherung erstellt.")?;
                result.files.push((child, metadata.len()));
                if result.files.len() > MAX_FILES {
                    return Err(
                        "Die Instanz enthält zu viele Dateien für eine vollständige Sicherung."
                            .into(),
                    );
                }
            } else {
                return Err("Sonderdateien und symbolische Links können nicht in eine sichere Instanzsicherung aufgenommen werden.".into());
            }
        }
        Ok(())
    }
    let mut result = Inventory::default();
    let target = path(root, base)?;
    if !target.exists() {
        return Ok(result);
    }
    walk(root, base, Path::new(""), 0, &mut result)?;
    Ok(result)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn unique() -> String {
    format!(
        "{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        std::process::id()
    )
}

pub(crate) fn ensure_no_pending_restore(root: &Path, instance: &str) -> Result<(), String> {
    identifier(instance)?;
    if path(
        root,
        &PathBuf::from("restore-transactions").join(format!("{instance}.json")),
    )?
    .exists()
    {
        return Err("Eine Sicherungswiederherstellung wurde unterbrochen. Wähle die Sicherung erneut aus, um den Vorgang sicher zu wiederholen.".into());
    }
    Ok(())
}

/// Caller holds INSTALL_LOCK and CONTENT_LOCK and has checked that Minecraft is stopped.
pub(crate) fn create_at(
    root: &Path,
    instance: &str,
    profile: Value,
    reason: &str,
) -> Result<BackupSummary, String> {
    ensure_no_pending_restore(root, instance)?;
    let profile = normalize_profile(profile, instance)?;
    let reason = if reason.is_empty() { "manual" } else { reason };
    text(reason, 100, "Sicherungsgrund")?;
    let source = PathBuf::from("instances").join(instance);
    let inventory = inventory(root, &source)?;
    let mut names = BTreeSet::new();
    for relative in inventory
        .directories
        .iter()
        .chain(inventory.files.iter().map(|(file, _)| file))
    {
        if !names.insert(posix(relative)?.to_lowercase()) {
            return Err("Die Instanz enthält Dateinamen, die nur durch Groß-/Kleinschreibung verschieden sind. Eine eindeutige Sicherung ist nicht möglich.".into());
        }
    }
    let contents: Vec<ContentSelection> = serde_json::from_value(profile["contents"].clone())
        .map_err(|_| "Ungültige Inhalte im Sicherungsprofil.")?;
    let verified = content::verify_installation(
        root,
        instance,
        profile["mcVersion"].as_str().unwrap(),
        profile["loader"].as_str().unwrap(),
        &contents,
    )
    .is_ok();
    let id = format!("backup-{}", unique());
    let final_relative = backup_base(instance, &id)?;
    let stage_relative = PathBuf::from("backups")
        .join(instance)
        .join(format!("pending-{id}"));
    let stage = path(root, &stage_relative)?;
    fs::create_dir_all(stage.join("data"))
        .map_err(|e| format!("Der Sicherungsordner konnte nicht erstellt werden: {e}"))?;
    let result = (|| {
        let mut files = Vec::new();
        let mut directories = Vec::new();
        let mut sections = BTreeSet::from(["profile".to_owned()]);
        if !inventory.files.is_empty() {
            sections.insert("instance-files".into());
        }
        let mut bytes = 0_u64;
        for directory in &inventory.directories {
            let target = path(root, &stage_relative.join("data").join(directory))?;
            fs::create_dir_all(target).map_err(|e| e.to_string())?;
            directories.push(posix(directory)?);
        }
        for (relative, size) in &inventory.files {
            let input = path(root, &source.join(relative))?;
            let output = path(root, &stage_relative.join("data").join(relative))?;
            let (written, hash) = copy_verified(&input, &output, None)?;
            if written != *size {
                return Err("Die Instanz wurde während der Sicherung verändert.".into());
            }
            bytes = bytes
                .checked_add(written)
                .filter(|size| *size <= MAX_BYTES)
                .ok_or("Die Sicherung überschreitet 20 GiB.")?;
            let name = posix(relative)?;
            match name.split('/').next().unwrap_or("") {
                "saves" => {
                    sections.insert("worlds".into());
                }
                "config" | "defaultconfigs" => {
                    sections.insert("config".into());
                }
                "mods" => {
                    sections.insert("mods".into());
                }
                "resourcepacks" => {
                    sections.insert("resourcepacks".into());
                }
                "shaderpacks" => {
                    sections.insert("shaderpacks".into());
                }
                "options.txt" | "optionsof.txt" => {
                    sections.insert("options".into());
                }
                "servers.dat" | "servers.dat_old" => {
                    sections.insert("servers".into());
                }
                _ => {}
            }
            files.push(BackupFile {
                path: name,
                sha512: hash,
                size: written,
            });
        }
        let summary = BackupSummary {
            id: id.clone(),
            instance_id: instance.into(),
            created_at: now(),
            reason: reason.into(),
            file_count: files.len(),
            total_bytes: bytes,
            mc_version: profile["mcVersion"].as_str().unwrap().into(),
            loader: profile["loader"].as_str().unwrap().into(),
            profile_name: profile["name"].as_str().unwrap().into(),
            contents_count: contents.len(),
            verified_content: verified,
            contained_sections: sections.into_iter().collect(),
        };
        let manifest = Manifest {
            format: "syncmc-instance-backup".into(),
            version: 1,
            summary: summary.clone(),
            profile,
            directories,
            files,
        };
        if serde_json::to_vec_pretty(&manifest)
            .map_err(|e| e.to_string())?
            .len() as u64
            > MAX_MANIFEST_BYTES
        {
            return Err("Das vollständige Sicherungsmanifest überschreitet 64 MiB. Es wurde keine unvollständige Sicherung erstellt.".into());
        }
        if verified {
            content::verify_snapshot_contents(&stage.join("data"),&manifest.profile).map_err(|_|"Die Inhalte wurden während der Sicherung verändert. Die Sicherung wurde nicht als vollständig gespeichert.")?;
        }
        write_json(root, &stage_relative.join("manifest.json"), &manifest)?;
        let final_path = path(root, &final_relative)?;
        fs::rename(&stage, final_path).map_err(|e| {
            format!("Die vollständige Sicherung konnte nicht abgeschlossen werden: {e}")
        })?;
        Ok(summary)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(stage);
    }
    result
}

pub(crate) fn list_at(root: &Path, instance: &str) -> Result<Vec<BackupSummary>, String> {
    identifier(instance)?;
    let base = path(root, &PathBuf::from("backups").join(instance))?;
    let entries = match fs::read_dir(base) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
    };
    let mut summaries = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let id = entry
            .file_name()
            .into_string()
            .map_err(|_| "Ungültiger Sicherungsname.")?;
        if id.starts_with("pending-") {
            continue;
        }
        if !id.starts_with("backup-") {
            continue;
        }
        summaries.push(read_manifest(root, instance, &id)?.summary);
    }
    summaries.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.id.cmp(&a.id))
    });
    Ok(summaries)
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RestoreTransaction {
    format: String,
    version: u8,
    instance_id: String,
    stage: String,
    previous: String,
    #[serde(default)]
    committed: bool,
}

fn transaction_relative(instance: &str) -> Result<PathBuf, String> {
    identifier(instance)?;
    Ok(PathBuf::from("restore-transactions").join(format!("{instance}.json")))
}

/// Always roll an interrupted swap back to its original live folder before attempting again.
fn recover_at(root: &Path, instance: &str) -> Result<(), String> {
    let journal = path(root, &transaction_relative(instance)?)?;
    if !journal.exists() {
        return Ok(());
    }
    let file = File::open(&journal).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > 4096 {
        return Err("Ungültiges Wiederherstellungsjournal.".into());
    }
    let tx: RestoreTransaction = serde_json::from_reader(file)
        .map_err(|_| "Das Wiederherstellungsjournal ist beschädigt.")?;
    if tx.format != "syncmc-restore-transaction" || tx.version != 1 || tx.instance_id != instance {
        return Err("Ungültiges Wiederherstellungsjournal.".into());
    }
    let stage_relative = from_posix(&tx.stage)?;
    let previous_relative = from_posix(&tx.previous)?;
    // Only our generated transaction directories can ever be moved or removed.
    if stage_relative.components().count() != 2
        || previous_relative.components().count() != 2
        || !tx.stage.starts_with("restore-staging/stage-")
        || !tx.previous.starts_with("restore-staging/previous-")
    {
        return Err("Unsichere Pfade im Wiederherstellungsjournal.".into());
    }
    let stage = path(root, &stage_relative)?;
    let previous = path(root, &previous_relative)?;
    let live = path(root, &PathBuf::from("instances").join(instance))?;
    if tx.committed {
        if !live.exists() {
            return Err("Der fertig wiederhergestellte Instanzordner fehlt. Bitte eine Rettungssicherung auswählen.".into());
        }
        if previous.exists() {
            fs::remove_dir_all(&previous).map_err(|e|format!("Der abgeschlossene Wiederherstellungsvorgang konnte nicht bereinigt werden: {e}"))?;
        }
        if stage.exists() {
            return Err("Ein abgeschlossener Wiederherstellungsvorgang enthält einen unerwarteten Stage-Ordner. Die Daten bleiben erhalten.".into());
        }
        return fs::remove_file(journal).map_err(|e| e.to_string());
    }
    if previous.exists() {
        if live.exists() {
            if stage.exists() {
                return Err("Die unterbrochene Wiederherstellung enthält widersprüchliche Ordner. Die Dateien bleiben zur manuellen Rettung erhalten.".into());
            }
            fs::rename(&live, &stage).map_err(|e| {
                format!("Der unvollständige Stand konnte nicht gesichert werden: {e}")
            })?;
        }
        fs::rename(&previous, &live).map_err(|e| {
            format!("Der vorherige Instanzstand konnte nicht wiederhergestellt werden: {e}")
        })?;
    } else if !live.exists() {
        return Err("Der vorherige Instanzordner fehlt. Bitte eine Rettungssicherung auswählen; alle Sicherungsdateien bleiben erhalten.".into());
    }
    if stage.exists() {
        fs::remove_dir_all(&stage).map_err(|e| {
            format!(
                "Der zurückgerollte Wiederherstellungsordner konnte nicht bereinigt werden: {e}"
            )
        })?;
    }
    fs::remove_file(journal).map_err(|e| e.to_string())
}

fn verify_backup_data(
    root: &Path,
    instance: &str,
    backup: &str,
    manifest: &Manifest,
) -> Result<(), String> {
    let data = backup_base(instance, backup)?.join("data");
    for directory in &manifest.directories {
        let directory = path(root, &data.join(from_posix(directory)?))?;
        if !directory.is_dir() {
            return Err("Ein gesichertes Verzeichnis fehlt.".into());
        }
    }
    for file in &manifest.files {
        let input = path(root, &data.join(from_posix(&file.path)?))?;
        let (size, hash) = hash_file(&input)?;
        if size != file.size || !hash.eq_ignore_ascii_case(&file.sha512) {
            return Err(format!("Die Sicherungsdatei „{}“ fehlt oder ist beschädigt. Der aktuelle Instanzstand wurde nicht verändert.",file.path));
        }
    }
    Ok(())
}

fn installation_matches(stage: &Path, profile: &Value) -> bool {
    let Ok(file) = File::open(stage.join("installation.json")) else {
        return false;
    };
    if file.metadata().map_or(true, |m| m.len() > 1024 * 1024) {
        return false;
    }
    let Ok(report) = serde_json::from_reader::<_, Value>(file) else {
        return false;
    };
    report["versionId"] == profile["mcVersion"]
        && report["loader"].as_str().unwrap_or("vanilla")
            == profile["loader"].as_str().unwrap_or("")
        && profile
            .get("loaderVersion")
            .is_none_or(|version| report["loaderVersion"] == *version)
}

pub(crate) fn restore_at(
    root: &Path,
    instance: &str,
    backup: &str,
) -> Result<BackupRestoreReport, String> {
    identifier(instance)?;
    identifier(backup)?;
    recover_at(root, instance)?;
    let manifest = read_manifest(root, instance, backup)?;
    verify_backup_data(root, instance, backup, &manifest)?;
    let fallback = content::profile_for_snapshot(root, instance, Some(manifest.profile.clone()))?;
    let rescue = create_at(root, instance, fallback, "before-restore")?;
    let nonce = unique();
    let stage_relative = PathBuf::from("restore-staging").join(format!("stage-{nonce}"));
    let previous_relative = PathBuf::from("restore-staging").join(format!("previous-{nonce}"));
    let stage = path(root, &stage_relative)?;
    fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
    let staging = (|| {
        for directory in &manifest.directories {
            fs::create_dir_all(path(root, &stage_relative.join(from_posix(directory)?))?)
                .map_err(|e| e.to_string())?;
        }
        for file in &manifest.files {
            let relative = from_posix(&file.path)?;
            let source = path(
                root,
                &backup_base(instance, backup)?.join("data").join(&relative),
            )?;
            let destination = path(root, &stage_relative.join(&relative))?;
            copy_verified(&source, &destination, Some(file))?;
        }
        Ok::<_, String>(())
    })();
    if let Err(error) = staging {
        let _ = fs::remove_dir_all(&stage);
        return Err(error);
    }
    let requires_installation = !installation_matches(&stage, &manifest.profile);
    let requires_content_install =
        content::verify_snapshot_contents(&stage, &manifest.profile).is_err();
    if requires_content_install && manifest.summary.verified_content {
        let _ = fs::remove_dir_all(&stage);
        return Err("Die Sicherung enthält keinen gültigen Inhaltsnachweis für ihr Profil. Der aktuelle Instanzstand wurde nicht verändert.".into());
    }
    if requires_installation && stage.join("installation.json").exists() {
        fs::remove_file(stage.join("installation.json")).map_err(|e| e.to_string())?;
    }
    let mut tx = RestoreTransaction {
        format: "syncmc-restore-transaction".into(),
        version: 1,
        instance_id: instance.into(),
        stage: posix(&stage_relative)?,
        previous: posix(&previous_relative)?,
        committed: false,
    };
    write_json(root, &transaction_relative(instance)?, &tx)?;
    let live = path(root, &PathBuf::from("instances").join(instance))?;
    let previous = path(root, &previous_relative)?;
    fs::create_dir_all(live.parent().ok_or("Der Instanzpfad fehlt.")?)
        .map_err(|e| e.to_string())?;
    if live.exists() {
        fs::rename(&live,&previous).map_err(|e|format!("Der aktuelle Instanzordner konnte nicht für die Wiederherstellung gesichert werden: {e}"))?;
    }
    if let Err(error) = fs::rename(&stage, &live) {
        let _ = recover_at(root, instance);
        return Err(format!("Die Sicherung konnte nicht aktiviert werden: {error}. Der vorherige Stand wird erhalten."));
    }
    tx.committed = true;
    write_json(root, &transaction_relative(instance)?, &tx)?;
    // The verified rescue snapshot makes cleanup of this transaction copy reversible.
    if previous.exists() {
        fs::remove_dir_all(&previous).map_err(|e|format!("Der Wiederherstellungsvorgang konnte nicht abgeschlossen werden: {e}. Bitte die Sicherung erneut auswählen."))?;
    }
    fs::remove_file(path(root, &transaction_relative(instance)?)?).map_err(|e| e.to_string())?;
    let contents = serde_json::from_value(manifest.profile["contents"].clone())
        .map_err(|_| "Die gesicherten Inhalte sind ungültig.")?;
    Ok(BackupRestoreReport {
        backup: manifest.summary,
        rescue_backup: rescue,
        profile: manifest.profile,
        contents,
        requires_installation,
        requires_content_install,
    })
}

#[tauri::command]
pub async fn create_instance_backup(
    app: tauri::AppHandle,
    instance_id: String,
    profile: Value,
    reason: Option<String>,
) -> Result<BackupSummary, String> {
    let _install = crate::minecraft::INSTALL_LOCK
        .try_lock()
        .map_err(|_| "Eine Installation oder ein Spielstart läuft bereits.")?;
    let _content = content::CONTENT_LOCK
        .try_lock()
        .map_err(|_| "Eine Inhaltsänderung oder Sicherung läuft bereits.")?;
    crate::launch::ensure_stopped()?;
    let root = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("minecraft");
    tauri::async_runtime::spawn_blocking(move || {
        let profile = content::profile_for_snapshot(&root, &instance_id, Some(profile))?;
        create_at(
            &root,
            &instance_id,
            profile,
            reason.as_deref().unwrap_or("manual"),
        )
    })
    .await
    .map_err(|_| "Der Sicherungsvorgang wurde unterbrochen.")?
}

#[tauri::command]
pub async fn list_instance_backups(
    app: tauri::AppHandle,
    instance_id: String,
) -> Result<Vec<BackupSummary>, String> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("minecraft");
    tauri::async_runtime::spawn_blocking(move || list_at(&root, &instance_id))
        .await
        .map_err(|_| "Die Sicherungsliste konnte nicht gelesen werden.")?
}

#[tauri::command]
pub async fn restore_instance_backup(
    app: tauri::AppHandle,
    instance_id: String,
    backup_id: String,
) -> Result<BackupRestoreReport, String> {
    let _install = crate::minecraft::INSTALL_LOCK
        .try_lock()
        .map_err(|_| "Eine Installation oder ein Spielstart läuft bereits.")?;
    let _content = content::CONTENT_LOCK
        .try_lock()
        .map_err(|_| "Eine Inhaltsänderung oder Sicherung läuft bereits.")?;
    crate::launch::ensure_stopped()?;
    let root = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("minecraft");
    tauri::async_runtime::spawn_blocking(move || restore_at(&root, &instance_id, &backup_id))
        .await
        .map_err(|_| {
            "Die Wiederherstellung wurde unterbrochen. Bitte die Sicherung erneut auswählen."
        })?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    fn temp() -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "syncmc-backup-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }
    fn profile() -> Value {
        default_profile("test", "1.21.1", "fabric", &[])
    }
    fn live_file(root: &Path, name: &str, bytes: &[u8]) {
        let target = path(root, &PathBuf::from("instances/test").join(name)).unwrap();
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, bytes).unwrap();
    }
    #[test]
    fn snapshot_restores_world_config_and_profile_and_rescues_newer_world() {
        let root = temp();
        live_file(&root, "saves/World/region/r.0.0.mca", b"old-world");
        live_file(&root, "config/mod.json", b"old-config");
        live_file(&root, "options.txt", b"old-options");
        live_file(&root, "servers.dat", b"old-servers");
        let backup = create_at(&root, "test", profile(), "manual").unwrap();
        assert!(backup.contained_sections.contains(&"worlds".into()));
        assert_eq!(list_at(&root, "test").unwrap().len(), 1);
        live_file(&root, "saves/World/region/r.0.0.mca", b"new-world");
        live_file(&root, "config/new.json", b"new-file");
        let restored = restore_at(&root, "test", &backup.id).unwrap();
        assert_eq!(
            fs::read(root.join("instances/test/saves/World/region/r.0.0.mca")).unwrap(),
            b"old-world"
        );
        assert!(!root.join("instances/test/config/new.json").exists());
        assert_eq!(restored.profile["mcVersion"], "1.21.1");
        assert!(restored.requires_installation);
        restore_at(&root, "test", &restored.rescue_backup.id).unwrap();
        assert_eq!(
            fs::read(root.join("instances/test/saves/World/region/r.0.0.mca")).unwrap(),
            b"new-world"
        );
        assert!(root.join("instances/test/config/new.json").exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn damaged_backup_never_changes_live_instance() {
        let root = temp();
        live_file(&root, "options.txt", b"before");
        let backup = create_at(&root, "test", profile(), "manual").unwrap();
        fs::write(
            root.join("backups/test")
                .join(&backup.id)
                .join("data/options.txt"),
            b"corrupt",
        )
        .unwrap();
        live_file(&root, "options.txt", b"current");
        assert!(restore_at(&root, "test", &backup.id).is_err());
        assert_eq!(
            fs::read(root.join("instances/test/options.txt")).unwrap(),
            b"current"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn traversal_and_oversize_manifest_are_rejected() {
        for name in ["../test", "/test", "test\\evil"] {
            assert!(identifier(name).is_err());
        }
        for relative in [
            "../options.txt",
            "/options.txt",
            "mods/../secret",
            "mods\\secret",
        ] {
            assert!(from_posix(relative).is_err());
        }
        let root = temp();
        live_file(&root, "options.txt", b"bytes");
        let backup = create_at(&root, "test", profile(), "manual").unwrap();
        let target = root
            .join("backups/test")
            .join(&backup.id)
            .join("manifest.json");
        let mut manifest: Value = serde_json::from_slice(&fs::read(&target).unwrap()).unwrap();
        manifest["files"][0]["path"] = json!("../../foreign");
        fs::write(&target, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(restore_at(&root, "test", &backup.id).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn incomplete_directory_swap_rolls_back_before_retry() {
        let root = temp();
        live_file(&root, "options.txt", b"live");
        let stage_relative = PathBuf::from("restore-staging/stage-test");
        let previous_relative = PathBuf::from("restore-staging/previous-test");
        fs::create_dir_all(root.join(&stage_relative)).unwrap();
        fs::write(root.join(&stage_relative).join("options.txt"), b"restored").unwrap();
        let tx = RestoreTransaction {
            format: "syncmc-restore-transaction".into(),
            version: 1,
            instance_id: "test".into(),
            stage: posix(&stage_relative).unwrap(),
            previous: posix(&previous_relative).unwrap(),
            committed: false,
        };
        write_json(&root, &transaction_relative("test").unwrap(), &tx).unwrap();
        fs::rename(root.join("instances/test"), root.join(&previous_relative)).unwrap();
        assert!(ensure_no_pending_restore(&root, "test").is_err());
        recover_at(&root, "test").unwrap();
        assert_eq!(
            fs::read(root.join("instances/test/options.txt")).unwrap(),
            b"live"
        );
        assert!(!root.join(&stage_relative).exists());
        ensure_no_pending_restore(&root, "test").unwrap();
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn profile_never_copies_credentials() {
        let mut profile = profile();
        profile["accessToken"] = json!("secret");
        profile["device"] = json!({"microsoftClientId":"private-device-setting"});
        let normalized = normalize_profile(profile, "test").unwrap();
        assert!(normalized.get("accessToken").is_none());
        assert!(normalized.get("device").is_none());
        let wrong = default_profile("other", "1.21.1", "fabric", &[]);
        assert!(normalize_profile(wrong, "test").is_err());
    }
    #[cfg(unix)]
    #[test]
    fn symlink_and_size_limit_fail_without_completed_backup() {
        let root = temp();
        let outside = temp();
        live_file(&root, "options.txt", b"safe");
        std::os::unix::fs::symlink(&outside, root.join("instances/test/config")).unwrap();
        assert!(create_at(&root, "test", profile(), "manual").is_err());
        assert!(list_at(&root, "test").unwrap().is_empty());
        fs::remove_file(root.join("instances/test/config")).unwrap();
        let big = File::create(root.join("instances/test/huge.dat")).unwrap();
        big.set_len(MAX_BYTES + 1).unwrap();
        assert!(create_at(&root, "test", profile(), "manual").is_err());
        assert!(list_at(&root, "test").unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }
}

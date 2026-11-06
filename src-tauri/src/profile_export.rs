//! Native local export for WebViews that do not implement Blob downloads reliably.
//! Recovery text is intentionally copied verbatim; it need not be valid JSON.
use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tauri::Manager;
use uuid::Uuid;

const MAX_BYTES: usize = 5_000_000;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReport {
    pub path: String,
    pub bytes: u64,
}

fn filename_stem(filename: &str) -> Result<&str, String> {
    let suffix = filename
        .strip_prefix("syncmc-")
        .and_then(|name| name.strip_suffix(".json"));
    if filename.len() > 160
        || suffix.is_none_or(|name| {
            name.is_empty()
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        })
    {
        return Err("Der Exportdateiname muss syncmc-Name.json verwenden und darf nur Buchstaben, Ziffern, Bindestriche und Unterstriche enthalten.".into());
    }
    Ok(&filename[..filename.len() - ".json".len()])
}

/// Check every existing ancestor before creating the fixed destination, including the app-data
/// directory itself. No input from the WebView is ever interpreted as a directory or URL.
fn no_symlink_ancestors(target: &Path) -> Result<(), String> {
    if !target.is_absolute() {
        return Err("Der lokale Exportordner hat keinen absoluten Pfad.".into());
    }
    for ancestor in target.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("Der Exportordner darf keinen symbolischen Link enthalten.".into())
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err("Ein Bestandteil des Exportpfads ist kein Ordner.".into())
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "Der Exportpfad konnte nicht geprüft werden: {error}"
                ))
            }
        }
    }
    Ok(())
}

fn exports_directory(app_data: &Path) -> Result<PathBuf, String> {
    no_symlink_ancestors(app_data)?;
    fs::create_dir_all(app_data)
        .map_err(|error| format!("Der SyncMC-Datenordner konnte nicht erstellt werden: {error}"))?;
    let exports = app_data.join("exports");
    no_symlink_ancestors(&exports)?;
    if !exports.exists() {
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(&exports) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(format!(
                    "Der Exportordner konnte nicht erstellt werden: {error}"
                ))
            }
        }
    }
    no_symlink_ancestors(&exports)?;
    Ok(exports)
}

pub(crate) fn save_at(
    app_data: &Path,
    contents: &str,
    filename: &str,
) -> Result<ExportReport, String> {
    let stem = filename_stem(filename)?;
    let bytes = contents.as_bytes();
    if bytes.len() > MAX_BYTES {
        return Err(
            "Der Profilexport überschreitet die Grenze von 5 MB. Es wurde keine Datei gespeichert."
                .into(),
        );
    }
    let directory = exports_directory(app_data)?;
    let temporary = directory.join(format!(".syncmc-export-{}.part", Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut output = options
        .open(&temporary)
        .map_err(|error| format!("Die Exportdatei konnte nicht angelegt werden: {error}"))?;
    let result = (|| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            output
                .set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|error| {
                    format!("Die privaten Exportdateirechte konnten nicht gesetzt werden: {error}")
                })?;
        }
        output.write_all(bytes).map_err(|error| {
            format!("Die Profile konnten nicht vollständig exportiert werden: {error}")
        })?;
        output.sync_all().map_err(|error| {
            format!("Die Exportdatei konnte nicht dauerhaft gespeichert werden: {error}")
        })?;
        if output.metadata().map_err(|error| error.to_string())?.len() != bytes.len() as u64 {
            return Err(
                "Die Exportdatei ist unvollständig. Es wurde kein fertiger Export erstellt.".into(),
            );
        }
        // A hard link publishes the already complete inode atomically and fails if the name
        // exists. Unlike rename, this can never overwrite another export on Unix.
        for _ in 0..8 {
            let destination = directory.join(format!("{stem}-{}.json", Uuid::new_v4()));
            match fs::hard_link(&temporary, &destination) {
                Ok(()) => return Ok(ExportReport { path: destination.to_string_lossy().into_owned(), bytes: bytes.len() as u64 }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(format!("Der fertige Profilexport konnte nicht angelegt werden: {error}. Bestehende Exporte wurden nicht verändert.")),
            }
        }
        Err("Es konnte kein eindeutiger Exportdateiname angelegt werden. Bestehende Exporte bleiben erhalten.".into())
    })();
    drop(output);
    // On success the completed file has its own directory entry; removing this work name
    // cannot affect it. A failed publication never leaves a misleading *.json file behind.
    let _ = fs::remove_file(&temporary);
    result
}

#[tauri::command]
pub async fn save_profile_export(
    app: tauri::AppHandle,
    contents: String,
    filename: String,
) -> Result<ExportReport, String> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Der SyncMC-Datenordner konnte nicht gefunden werden: {error}"))?;
    tauri::async_runtime::spawn_blocking(move || save_at(&app_data, &contents, &filename))
        .await
        .map_err(|_| "Der Profilexport wurde unterbrochen. Bitte erneut versuchen.")?
}

#[tauri::command]
pub async fn open_profile_exports(app: tauri::AppHandle) -> Result<(), String> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Der SyncMC-Datenordner konnte nicht gefunden werden: {error}"))?;
    tauri::async_runtime::spawn_blocking(move || {
        let directory = exports_directory(&app_data)?;
        #[cfg(target_os = "macos")]
        let mut command = {
            let mut command = Command::new("/usr/bin/open");
            command.arg("--");
            command
        };
        #[cfg(target_os = "windows")]
        let mut command = {
            let mut command = Command::new("explorer.exe");
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
            command
        };
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let mut command = Command::new("xdg-open");
        command
            .arg(directory)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
            .spawn()
            .map_err(|_| "Der Exportordner konnte nicht im Dateimanager geöffnet werden.")?;
        Ok(())
    })
    .await
    .map_err(|_| "Das Öffnen des Exportordners wurde unterbrochen.")?
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temp(PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn temp() -> Temp {
        let base = fs::canonicalize(std::env::temp_dir()).unwrap();
        let root = base.join(format!("syncmc-profile-export-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        Temp(root)
    }

    #[test]
    fn copies_raw_recovery_bytes_without_parsing_and_never_overwrites() {
        let root = temp();
        let existing = root.0.join("exports");
        fs::create_dir(&existing).unwrap();
        fs::write(existing.join("syncmc-test.json"), b"older export").unwrap();
        let raw = "{beschädigter Rohstand\nmit Umlauten: äöü und 🧱";
        let first = save_at(&root.0, raw, "syncmc-test.json").unwrap();
        let second = save_at(&root.0, raw, "syncmc-test.json").unwrap();
        assert_ne!(first.path, second.path);
        assert_eq!(first.bytes, raw.len() as u64);
        assert_eq!(fs::read(&first.path).unwrap(), raw.as_bytes());
        assert_eq!(fs::read(&second.path).unwrap(), raw.as_bytes());
        assert_eq!(
            fs::read(existing.join("syncmc-test.json")).unwrap(),
            b"older export"
        );
        assert_eq!(
            fs::read_dir(existing).unwrap().count(),
            3,
            "no unfinished work files remain"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(first.path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn checks_utf8_byte_limit_and_preserves_previous_files_on_failure() {
        let root = temp();
        let existing = save_at(&root.0, "keep", "syncmc-instanzen.json").unwrap();
        let maximum = "x".repeat(MAX_BYTES);
        let report = save_at(&root.0, &maximum, "syncmc-max.json").unwrap();
        assert_eq!(report.bytes, MAX_BYTES as u64);
        assert!(save_at(&root.0, &(maximum + "x"), "syncmc-large.json").is_err());
        let multibyte = "ä".repeat(MAX_BYTES / 2 + 1);
        assert!(save_at(&root.0, &multibyte, "syncmc-large.json").is_err());
        assert_eq!(fs::read(existing.path).unwrap(), b"keep");
        assert_eq!(fs::read_dir(root.0.join("exports")).unwrap().count(), 2);
    }

    #[test]
    fn accepts_profile_backup_names_and_rejects_paths_urls_and_other_names() {
        filename_stem("syncmc-profilsicherung-profiles-a123-b456.json").unwrap();
        for name in [
            "../syncmc-a.json",
            "/syncmc-a.json",
            "syncmc-../x.json",
            "syncmc-a\\b.json",
            "syncmc-.json",
            "syncmc-a.json.exe",
            "other.json",
            "https://example.com/syncmc-a.json",
            "syncmc-a.json ",
            "syncmc-ä.json",
            "syncmc-a:b.json",
        ] {
            assert!(filename_stem(name).is_err(), "{name}");
        }
        assert!(filename_stem(&format!("syncmc-{}.json", "x".repeat(200))).is_err());
        let root = temp();
        assert!(save_at(&root.0, "data", "../syncmc-a.json").is_err());
        assert!(!root.0.join("exports").exists());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinked_appdata_and_exports_without_touching_the_target() {
        let root = temp();
        let outside = temp();
        std::os::unix::fs::symlink(&outside.0, root.0.join("exports")).unwrap();
        assert!(save_at(&root.0, "raw", "syncmc-test.json").is_err());
        assert!(fs::read_dir(&outside.0).unwrap().next().is_none());
        let alias = root.0.join("appdata-link");
        std::os::unix::fs::symlink(&outside.0, &alias).unwrap();
        assert!(save_at(&alias, "raw", "syncmc-test.json").is_err());
        assert!(fs::read_dir(&outside.0).unwrap().next().is_none());
    }
}

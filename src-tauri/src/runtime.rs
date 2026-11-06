use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tauri::Manager;
use tokio::process::Command;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInfo {
    pub os: &'static str,
    pub arch: &'static str,
    pub java_available: bool,
    pub java_version: Option<String>,
    pub java_major: Option<u32>,
    pub java_arch: Option<String>,
    pub java_path: String,
}

fn normalize_arch(arch: &str) -> &str {
    match arch {
        "arm64" | "aarch64" => "aarch64",
        "amd64" | "x86_64" => "x86_64",
        "i386" | "i686" | "x86" => "x86",
        other => other,
    }
}

fn java_major(version: &str) -> Option<u32> {
    let mut parts = version.split(['.', '-', '+', '_']);
    match parts.next()? {
        "1" => parts.next()?.parse().ok(),
        major => major.parse().ok(),
    }
}

fn property<'a>(output: &'a str, name: &str) -> Option<&'a str> {
    output
        .lines()
        .filter_map(|line| line.trim().split_once(" = "))
        .find_map(|(key, value)| (key == name).then_some(value.trim()))
}

pub async fn inspect(java_path: &str) -> RuntimeInfo {
    let path = if java_path.trim().is_empty() {
        "java"
    } else {
        java_path.trim()
    };
    let mut info = RuntimeInfo {
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        java_available: false,
        java_version: None,
        java_major: None,
        java_arch: None,
        java_path: path.into(),
    };
    if path.len() > 4096 || path.contains(['\0', '\n', '\r']) {
        return info;
    }
    let mut command = Command::new(path);
    command
        .args(["-XshowSettings:properties", "-version"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // Ignore ambient JVM injection: the selected binary is the entire command.
    command
        .env_remove("JAVA_TOOL_OPTIONS")
        .env_remove("JDK_JAVA_OPTIONS")
        .env_remove("_JAVA_OPTIONS");
    #[cfg(target_os = "windows")]
    command.creation_flags(0x08000000);
    if let Ok(Ok(output)) = tokio::time::timeout(Duration::from_secs(8), command.output()).await {
        if output.status.success() && output.stderr.len() + output.stdout.len() < 128 * 1024 {
            let settings = format!(
                "{}\n{}",
                String::from_utf8_lossy(&output.stderr),
                String::from_utf8_lossy(&output.stdout)
            );
            info.java_version = property(&settings, "java.version").map(str::to_owned);
            info.java_major = info.java_version.as_deref().and_then(java_major);
            info.java_arch =
                property(&settings, "os.arch").map(|arch| normalize_arch(arch).to_owned());
            if let Some(home) = property(&settings, "java.home") {
                let binary = Path::new(home).join("bin").join(if cfg!(windows) {
                    "java.exe"
                } else {
                    "java"
                });
                if binary.is_file() {
                    info.java_path = binary.to_string_lossy().into();
                }
            }
            info.java_available = info.java_major.is_some() && info.java_arch.is_some();
        }
    }
    info
}

#[tauri::command]
pub async fn runtime_info(java_path: Option<String>) -> RuntimeInfo {
    inspect(java_path.as_deref().unwrap_or("")).await
}

fn candidates() -> Vec<PathBuf> {
    let executable = if cfg!(windows) { "java.exe" } else { "java" };
    let mut result = Vec::new();
    if let Some(home) = std::env::var_os("JAVA_HOME") {
        result.push(PathBuf::from(home).join("bin").join(executable));
    }
    #[cfg(target_os = "macos")]
    for root in [
        "/Library/Java/JavaVirtualMachines",
        "/opt/homebrew/opt",
        "/usr/local/opt",
    ] {
        if let Ok(entries) = std::fs::read_dir(root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if root.ends_with("/opt")
                    && !entry.file_name().to_string_lossy().starts_with("openjdk")
                {
                    continue;
                }
                result.push(path.join("Contents/Home/bin/java"));
                result.push(path.join("libexec/openjdk.jdk/Contents/Home/bin/java"));
            }
        }
    }
    #[cfg(target_os = "linux")]
    if let Ok(entries) = std::fs::read_dir("/usr/lib/jvm") {
        result.extend(entries.flatten().map(|entry| entry.path().join("bin/java")));
    }
    #[cfg(target_os = "windows")]
    for root in [
        "C:\\Program Files\\Eclipse Adoptium",
        "C:\\Program Files\\Java",
        "C:\\Program Files\\Microsoft",
    ] {
        if let Ok(entries) = std::fs::read_dir(root) {
            result.extend(
                entries
                    .flatten()
                    .map(|entry| entry.path().join("bin/java.exe")),
            );
        }
    }
    result.retain(|path| path.is_file());
    result
}

pub async fn validate(java_path: &str, required_major: u32) -> Result<RuntimeInfo, String> {
    let mut info = inspect(java_path).await;
    if java_path.trim().is_empty()
        && (info.java_major != Some(required_major)
            || info.java_arch.as_deref() != Some(normalize_arch(std::env::consts::ARCH)))
    {
        for candidate in candidates() {
            let found = inspect(&candidate.to_string_lossy()).await;
            if found.java_major == Some(required_major)
                && found.java_arch.as_deref() == Some(normalize_arch(std::env::consts::ARCH))
            {
                info = found;
                break;
            }
        }
    }
    if !info.java_available {
        return Err("Java wurde nicht gefunden. Installiere die benötigte Java-Version und wähle den Pfad zu bin/java in den Einstellungen.".into());
    }
    if info.java_major != Some(required_major) {
        return Err(format!("Diese Instanz benötigt Java {required_major}; ausgewählt ist Java {}. Bitte einen passenden Java-Pfad auswählen.", info.java_major.unwrap_or(0)));
    }
    if info.java_arch.as_deref() != Some(normalize_arch(std::env::consts::ARCH)) {
        return Err(format!("Java und die installierten Spielbibliotheken brauchen dieselbe Architektur ({}). Bitte ein passendes Java installieren.", std::env::consts::ARCH));
    }
    Ok(info)
}

/// Empty selection means automatic discovery, including verified app-local Temurin files.
/// An explicit selection always retains precedence and never triggers a download.
pub async fn validate_with_managed(
    java_path: &str,
    required_major: u32,
    app_data: &Path,
) -> Result<RuntimeInfo, String> {
    if java_path.trim().is_empty() {
        if let Some(path) = crate::managed_java::installed(app_data, required_major)? {
            return validate(&path.to_string_lossy(), required_major).await;
        }
    }
    validate(java_path, required_major).await
}

#[tauri::command]
pub async fn validate_java(
    app: tauri::AppHandle,
    java_path: String,
    required_major: u32,
) -> Result<RuntimeInfo, String> {
    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    validate_with_managed(&java_path, required_major, &app_data).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_java_versions_and_architectures() {
        assert_eq!(java_major("1.8.0_462"), Some(8));
        assert_eq!(java_major("21.0.8"), Some(21));
        assert_eq!(java_major("25-ea"), Some(25));
        assert_eq!(java_major("invalid"), None);
        assert_eq!(normalize_arch("arm64"), "aarch64");
        assert_eq!(
            property("  java.version = 21.0.8\n os.arch = aarch64", "os.arch"),
            Some("aarch64")
        );
    }
}

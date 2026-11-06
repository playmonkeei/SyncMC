//! Validated launch plans and a single supervised Minecraft process.
use crate::{auth, content, minecraft as mc, runtime};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{Emitter, Manager};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::sync::mpsc;

#[derive(Clone, Default, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameStatus {
    pub running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GameLog {
    pub timestamp: u64,
    pub stream: String,
    pub line: String,
}

#[derive(Default)]
struct GameState {
    status: GameStatus,
    starting: bool,
    logs: VecDeque<GameLog>,
    stop: Option<mpsc::Sender<bool>>,
}
static STATE: OnceLock<Mutex<GameState>> = OnceLock::new();
fn state() -> &'static Mutex<GameState> {
    STATE.get_or_init(|| Mutex::new(GameState::default()))
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn ensure_stopped() -> Result<(), String> {
    let state = state()
        .lock()
        .map_err(|_| "Spielstatus konnte nicht gelesen werden.")?;
    if state.status.running || state.starting {
        Err("Minecraft läuft bereits oder wird gerade gestartet. Beende das Spiel vor Änderungen an der Installation.".into())
    } else {
        Ok(())
    }
}

#[tauri::command]
pub fn game_status() -> Result<GameStatus, String> {
    Ok(state()
        .lock()
        .map_err(|_| "Spielstatus nicht verfügbar.")?
        .status
        .clone())
}
#[tauri::command]
pub fn get_game_logs() -> Result<Vec<GameLog>, String> {
    Ok(state()
        .lock()
        .map_err(|_| "Spielprotokoll nicht verfügbar.")?
        .logs
        .iter()
        .cloned()
        .collect())
}
#[tauri::command]
pub async fn stop_minecraft(force: Option<bool>) -> Result<(), String> {
    let stop = {
        state()
            .lock()
            .map_err(|_| "Spielstatus nicht verfügbar.")?
            .stop
            .clone()
    };
    if let Some(stop) = stop {
        let _ = stop.send(force.unwrap_or(false)).await;
    }
    Ok(())
}

struct Starting;
impl Drop for Starting {
    fn drop(&mut self) {
        if let Ok(mut s) = state().lock() {
            s.starting = false;
        }
    }
}

struct LaunchPlan {
    args: Vec<String>,
    directory: PathBuf,
}

fn read_json(root: &Path, relative: &Path) -> Result<Value, String> {
    let path = mc::managed_path(root, relative)?;
    if fs::metadata(&path)
        .map_err(|_| "Installation fehlt. Bitte Minecraft erneut installieren.")?
        .len()
        > 32 * 1024 * 1024
    {
        return Err("Installationsmetadaten sind zu groß.".into());
    }
    serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|_| "Installationsmetadaten sind beschädigt. Bitte erneut installieren.".into())
}

fn verified(root: &Path, item: &mc::Download) -> Result<(), String> {
    if !mc::verify_file(&mc::managed_path(root, &item.path)?, item)? {
        return Err(format!(
            "Spieldatei fehlt oder ist beschädigt: {}. Bitte die Installation erneut prüfen.",
            item.path.display()
        ));
    }
    Ok(())
}

fn expand(value: &str, variables: &BTreeMap<&str, String>) -> Result<String, String> {
    let mut result = value.to_owned();
    for (key, replacement) in variables {
        result = result.replace(&format!("${{{key}}}"), replacement);
    }
    if result.contains("${") || result.contains('\0') {
        return Err("Dieses Minecraft-Profil enthält nicht unterstützte Startparameter.".into());
    }
    Ok(result)
}

fn arguments(
    values: Option<&Value>,
    variables: &BTreeMap<&str, String>,
) -> Result<Vec<String>, String> {
    let Some(values) = values.and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    let mut args = Vec::new();
    let os_version = mc::host_version();
    for entry in values {
        if let Some(value) = entry.as_str() {
            args.push(expand(value, variables)?);
        } else if entry.is_object() {
            if !mc::rules_allow(
                entry.get("rules"),
                mc::minecraft_os(),
                mc::os_arch(),
                &os_version,
            ) {
                continue;
            }
            match &entry["value"] {
                Value::String(value) => args.push(expand(value, variables)?),
                Value::Array(values) => {
                    for value in values {
                        args.push(expand(
                            value.as_str().ok_or("Ungültiger Startparameter.")?,
                            variables,
                        )?);
                    }
                }
                _ => return Err("Ungültige Minecraft-Startparameter.".into()),
            }
        } else {
            return Err("Ungültige Minecraft-Startparameter.".into());
        }
    }
    Ok(args)
}

fn build_plan(
    root: &Path,
    instance_id: &str,
    version_id: &str,
    loader: &str,
    loader_version: Option<&str>,
    memory_mb: u32,
    fullscreen: bool,
    session: &auth::MinecraftSession,
) -> Result<(LaunchPlan, u32), String> {
    mc::valid_instance_id(instance_id)?;
    if !(1024..=32768).contains(&memory_mb) {
        return Err("Der Arbeitsspeicher muss zwischen 1024 und 32768 MB liegen.".into());
    }
    let report: mc::InstallationReport =
        serde_json::from_value(read_json(root, &mc::report_path(instance_id))?)
            .map_err(|_| "Installationsstatus ist ungültig.")?;
    if report.instance_id != instance_id
        || report.version_id != version_id
        || report.loader != loader
        || (loader == "fabric"
            && (loader_version.is_none() || report.loader_version.as_deref() != loader_version))
    {
        return Err("Die Installation passt nicht zum Instanzprofil. Bitte Minecraft und Modloader erneut installieren.".into());
    }
    let relative = PathBuf::from("instances").join(instance_id);
    let version = read_json(root, &relative.join("resolved-version.json"))?;
    if version["id"] != version_id {
        return Err("Falsche Minecraft-Startmetadaten. Bitte erneut installieren.".into());
    }
    let required_java = version["javaVersion"]["majorVersion"].as_u64().unwrap_or(8) as u32;
    let os_version = mc::host_version();
    let (libraries, natives) =
        mc::library_downloads(&version, mc::minecraft_os(), mc::os_arch(), &os_version)?;
    let mut classpath = Vec::new();
    // Only normal artifacts belong to the classpath. Old native classifier archives are extracted.
    for library in version["libraries"]
        .as_array()
        .ok_or("Bibliotheken fehlen.")?
    {
        if !mc::rules_allow(
            library.get("rules"),
            mc::minecraft_os(),
            mc::os_arch(),
            &os_version,
        ) {
            continue;
        }
        let name = library["name"].as_str().ok_or("Bibliotheksname fehlt.")?;
        if !mc::native_matches_arch(name, mc::os_arch()) {
            continue;
        }
        if let Some(artifact) = library["downloads"].get("artifact") {
            let relative =
                mc::relative_path(artifact["path"].as_str().ok_or("Bibliothekspfad fehlt.")?)?;
            classpath.push(mc::managed_path(
                root,
                &PathBuf::from("libraries").join(relative),
            )?);
        }
    }
    for library in &libraries {
        verified(root, library)?;
    }
    let client = mc::download_descriptor(
        &version["downloads"]["client"],
        PathBuf::from("versions")
            .join(mc::relative_path(version_id)?)
            .join(format!("{version_id}.jar")),
    )?;
    verified(root, &client)?;
    classpath.push(mc::managed_path(root, &client.path)?);
    let classpath = std::env::join_paths(classpath)
        .map_err(|_| "Klassenpfad konnte nicht erstellt werden.")?
        .to_string_lossy()
        .into_owned();
    let assets_id = version["assetIndex"]["id"]
        .as_str()
        .ok_or("Assetindex fehlt.")?;
    let assets_component = mc::relative_path(assets_id)?;
    if assets_component.components().count() != 1 {
        return Err("Ungültiger Assetindex.".into());
    }
    let index = mc::download_descriptor(
        &version["assetIndex"],
        PathBuf::from("assets/indexes").join(format!("{assets_id}.json")),
    )?;
    verified(root, &index)?;
    let asset_index = read_json(root, &index.path)?;
    // Verify the real bytes, not merely a persisted 'installed' flag.
    for asset in asset_index["objects"]
        .as_object()
        .ok_or("Assetindex ist ungültig.")?
        .values()
    {
        let hash = asset["hash"]
            .as_str()
            .filter(|h| h.len() == 40 && h.bytes().all(|c| c.is_ascii_hexdigit()))
            .ok_or("Ungültige Assetprüfsumme.")?;
        verified(
            root,
            &mc::Download {
                url: String::new(),
                sha1: hash.into(),
                size: asset["size"].as_u64(),
                path: PathBuf::from("assets/objects").join(&hash[..2]).join(hash),
            },
        )?;
    }
    let natives_relative = relative
        .join("natives")
        .join(mc::relative_path(version_id)?);
    mc::extract_natives(root, &natives, &natives_relative)?;
    let directory = mc::managed_path(root, &relative)?;
    let assets_root = root.join("assets");
    let native_directory = mc::managed_path(root, &natives_relative)?;
    let mut vars = BTreeMap::from([
        ("auth_player_name", session.name.clone()),
        ("auth_uuid", session.uuid.clone()),
        ("auth_access_token", session.access_token.clone()),
        (
            "auth_session",
            format!("token:{}:{}", session.access_token, session.uuid),
        ),
        ("auth_xuid", session.xuid.clone()),
        ("clientid", session.client_id.clone()),
        ("user_type", "msa".into()),
        ("user_properties", "{}".into()),
        ("version_name", version_id.into()),
        (
            "version_type",
            version["type"].as_str().unwrap_or("release").into(),
        ),
        ("game_directory", directory.to_string_lossy().into()),
        ("assets_root", assets_root.to_string_lossy().into()),
        ("assets_index_name", assets_id.into()),
        (
            "game_assets",
            if asset_index["virtual"] == true {
                assets_root
                    .join("virtual")
                    .join(&assets_component)
                    .to_string_lossy()
                    .into()
            } else {
                assets_root.to_string_lossy().into()
            },
        ),
        (
            "natives_directory",
            native_directory.to_string_lossy().into(),
        ),
        (
            "library_directory",
            root.join("libraries").to_string_lossy().into(),
        ),
        ("classpath", classpath.clone()),
        (
            "classpath_separator",
            if cfg!(windows) {
                ";".into()
            } else {
                ":".into()
            },
        ),
        ("launcher_name", "SyncMC".into()),
        ("launcher_version", env!("CARGO_PKG_VERSION").into()),
    ]);
    let mut args = vec!["-Xms512M".into(), format!("-Xmx{memory_mb}M")];
    let jvm = arguments(version["arguments"].get("jvm"), &vars)?;
    if jvm.is_empty() {
        if cfg!(target_os = "macos") {
            args.push("-XstartOnFirstThread".into());
        }
        args.extend([
            format!("-Djava.library.path={}", native_directory.display()),
            "-cp".into(),
            classpath,
        ]);
    } else {
        args.extend(jvm);
    }
    if let Some(logging) = version["logging"]["client"].get("file") {
        let id = mc::relative_path(logging["id"].as_str().ok_or("Loggingdatei fehlt.")?)?;
        let file = mc::download_descriptor(logging, PathBuf::from("assets/log_configs").join(id))?;
        verified(root, &file)?;
        vars.insert(
            "path",
            mc::managed_path(root, &file.path)?.to_string_lossy().into(),
        );
        if let Some(argument) = version["logging"]["client"]["argument"].as_str() {
            args.push(expand(argument, &vars)?);
        }
    }
    let main_class = version["mainClass"]
        .as_str()
        .filter(|s| {
            !s.is_empty()
                && s.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._$".contains(&c))
        })
        .ok_or("Ungültige Minecraft-Hauptklasse.")?;
    args.push(main_class.into());
    if version["arguments"]["game"].is_array() {
        args.extend(arguments(version["arguments"].get("game"), &vars)?);
    } else if let Some(legacy) = version["minecraftArguments"].as_str() {
        // Legacy official profiles contain simple whitespace-separated placeholders, not shell code.
        args.extend(
            legacy
                .split_whitespace()
                .map(|v| expand(v, &vars))
                .collect::<Result<Vec<_>, _>>()?,
        );
    } else {
        return Err("Minecraft-Spielargumente fehlen.".into());
    }
    if fullscreen {
        args.push("--fullscreen".into());
    }
    Ok((LaunchPlan { args, directory }, required_java))
}

fn push_log(
    app: &tauri::AppHandle,
    file: &Arc<Mutex<File>>,
    stream: &str,
    line: &str,
    secret: &str,
) {
    let line = if secret.is_empty() {
        line.to_owned()
    } else {
        line.replace(secret, "[redacted]")
    };
    let entry = GameLog {
        timestamp: now(),
        stream: stream.into(),
        line,
    };
    if let Ok(mut state) = state().lock() {
        if state.logs.len() == 1500 {
            state.logs.pop_front();
        }
        state.logs.push_back(entry.clone());
    }
    if let Ok(mut file) = file.lock() {
        if file.metadata().is_ok_and(|m| m.len() < 10 * 1024 * 1024) {
            let _ = writeln!(
                file,
                "{} [{}] {}",
                entry.timestamp, entry.stream, entry.line
            );
        }
    }
    let _ = app.emit("minecraft-game-log", entry);
}

async fn pipe_logs(
    mut pipe: impl AsyncRead + Unpin,
    app: tauri::AppHandle,
    file: Arc<Mutex<File>>,
    stream: &'static str,
    secret: String,
) {
    let mut buffer = [0u8; 4096];
    let mut pending = Vec::new();
    let mut discarding = false;
    loop {
        let count = match pipe.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(count) => count,
        };
        pending.extend_from_slice(&buffer[..count]);
        while let Some(newline) = pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = pending.drain(..=newline).collect();
            if !discarding {
                push_log(
                    &app,
                    &file,
                    stream,
                    String::from_utf8_lossy(&line).trim_end(),
                    &secret,
                );
            }
            discarding = false;
        }
        if pending.len() > 64 * 1024 {
            // Discard the entire pathological line, so no partial secret can escape redaction.
            push_log(&app, &file, stream, "[Zu lange Protokollzeile gekürzt]", "");
            pending.clear();
            discarding = true;
        }
    }
    if !discarding && !pending.is_empty() {
        push_log(
            &app,
            &file,
            stream,
            &String::from_utf8_lossy(&pending),
            &secret,
        );
    }
}

#[tauri::command]
pub async fn launch_minecraft(
    app: tauri::AppHandle,
    instance_id: String,
    version_id: String,
    loader: String,
    loader_version: Option<String>,
    java_path: String,
    memory_mb: u32,
    fullscreen: bool,
    contents: Vec<content::ContentSelection>,
) -> Result<GameStatus, String> {
    let _install = mc::INSTALL_LOCK
        .try_lock()
        .map_err(|_| "Eine Installation oder ein Spielstart läuft bereits.")?;
    let _content = content::CONTENT_LOCK
        .try_lock()
        .map_err(|_| "Modinstallation läuft noch.")?;
    ensure_stopped()?;
    state()
        .lock()
        .map_err(|_| "Spielstatus nicht verfügbar.")?
        .starting = true;
    let _starting = Starting;
    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let root = app_data.join("minecraft");
    let root = fs::canonicalize(root).map_err(|_| "Minecraft ist noch nicht installiert.")?;
    content::verify_installation(&root, &instance_id, &version_id, &loader, &contents)?;
    let session = auth::minecraft_session().await?;
    let (plan, required_java) = build_plan(
        &root,
        &instance_id,
        &version_id,
        &loader,
        loader_version.as_deref(),
        memory_mb,
        fullscreen,
        &session,
    )?;
    let java = runtime::validate_with_managed(&java_path, required_java, &app_data).await?;
    let log_relative = PathBuf::from("instances")
        .join(&instance_id)
        .join("logs/syncmc-launcher.log");
    let log_path = mc::managed_path(&root, &log_relative)?;
    fs::create_dir_all(log_path.parent().ok_or("Protokollpfad fehlt.")?)
        .map_err(|e| e.to_string())?;
    // Keep one bounded launcher log per run; Minecraft also maintains logs/latest.log itself.
    let file =
        Arc::new(Mutex::new(File::create(log_path).map_err(|e| {
            format!("Spielprotokoll konnte nicht erstellt werden: {e}")
        })?));
    let mut command = Command::new(&java.java_path);
    command
        .args(&plan.args)
        .current_dir(&plan.directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
        .env_remove("JAVA_TOOL_OPTIONS")
        .env_remove("JDK_JAVA_OPTIONS")
        .env_remove("_JAVA_OPTIONS");
    #[cfg(target_os = "windows")]
    command.creation_flags(0x08000000);
    let mut child = command
        .spawn()
        .map_err(|e| format!("Minecraft konnte nicht gestartet werden: {e}"))?;
    let stdout = child.stdout.take().ok_or("Minecraft-Ausgabe fehlt.")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("Minecraft-Fehlerausgabe fehlt.")?;
    let status = GameStatus {
        running: true,
        instance_id: Some(instance_id),
        pid: child.id(),
        exit_code: None,
        started_at: Some(now()),
    };
    let (stop, mut stopped) = mpsc::channel(4);
    {
        let mut state = state().lock().map_err(|_| "Spielstatus nicht verfügbar.")?;
        state.status = status.clone();
        state.logs.clear();
        state.stop = Some(stop);
    }
    let _ = app.emit("minecraft-game-status", &status);
    push_log(
        &app,
        &file,
        "launcher",
        &format!(
            "Minecraft {version_id} mit {loader} und Java {} gestartet.",
            java.java_major.unwrap_or(0)
        ),
        "",
    );
    let mut out = tauri::async_runtime::spawn(pipe_logs(
        stdout,
        app.clone(),
        file.clone(),
        "stdout",
        session.access_token.clone(),
    ));
    let mut err = tauri::async_runtime::spawn(pipe_logs(
        stderr,
        app.clone(),
        file.clone(),
        "stderr",
        session.access_token.clone(),
    ));
    tauri::async_runtime::spawn(async move {
        let result = loop {
            tokio::select! {
                result = child.wait() => break result,
                Some(force) = stopped.recv() => {
                    if force {
                        push_log(&app, &file, "launcher", "Erzwungenes Beenden angefordert.", "");
                        let _ = child.start_kill();
                    } else {
                        push_log(&app, &file, "launcher", "Beenden angefordert. Warte auf den Spielprozess; bitte möglichst im Spiel speichern und schließen.", "");
                        if let Some(pid) = child.id() {
                            #[cfg(unix)]
                            { unsafe { libc::kill(pid as i32, libc::SIGTERM); } }
                            #[cfg(target_os = "windows")]
                            { let _ = tokio::time::timeout(std::time::Duration::from_secs(5), Command::new("taskkill").args(["/PID", &pid.to_string()]).creation_flags(0x08000000).output()).await; }
                        }
                    }
                }
            }
        };
        // A mod subprocess can inherit a pipe and outlive Java. It must not pin 'running'.
        if tokio::time::timeout(std::time::Duration::from_secs(2), async {
            let _ = (&mut out).await;
            let _ = (&mut err).await;
        })
        .await
        .is_err()
        {
            out.abort();
            err.abort();
        }
        let exit_code = result.ok().and_then(|s| s.code()).unwrap_or(-1);
        push_log(
            &app,
            &file,
            "launcher",
            &format!("Minecraft wurde beendet (Exit-Code {exit_code})."),
            "",
        );
        let final_status = if let Ok(mut state) = state().lock() {
            state.status.running = false;
            state.status.exit_code = Some(exit_code);
            state.stop = None;
            state.status.clone()
        } else {
            GameStatus::default()
        };
        let _ = app.emit("minecraft-game-status", final_status);
    });
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn arguments_preserve_spaces_and_respect_rules() {
        let vars = BTreeMap::from([
            ("game_directory", "/some path/instance".into()),
            ("auth_access_token", "secret".into()),
        ]);
        let values = json!(["--gameDir", "${game_directory}", {"rules":[{"action":"allow","features":{"is_demo_user":true}}],"value":"--demo"}, {"rules":[{"action":"allow"}],"value":["--accessToken","${auth_access_token}"]}]);
        assert_eq!(
            arguments(Some(&values), &vars).unwrap(),
            [
                "--gameDir",
                "/some path/instance",
                "--accessToken",
                "secret"
            ]
        );
        assert!(expand("${missing}", &vars).is_err());
        assert!(expand("bad\0argument", &vars).is_err());
    }

    #[test]
    #[ignore = "Downloads Minecraft 1.21.1 and Fabric, verifies Java and a launch plan; no account or game login."]
    fn real_fabric_launch_preparation() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let root = PathBuf::from("/private/tmp/syncmc-fabric-smoke");
            let report = mc::install_at(
                &root,
                "fabric-smoke",
                "1.21.1",
                "fabric",
                None,
                Arc::new(|p| {
                    eprintln!("INSTALL_PROGRESS {}", serde_json::to_string(&p).unwrap());
                }),
            )
            .await
            .unwrap();
            assert_eq!(report.loader, "fabric");
            assert!(report.loader_version.is_some());
            let java = runtime::validate("", report.java_major_version as u32)
                .await
                .unwrap();
            assert_eq!(java.java_major, Some(21));
            // Synthetic credentials are confined to this plan test; no game is launched.
            let fixture = auth::MinecraftSession {
                name: "PlanTest".into(),
                uuid: "1234567890abcdef1234567890abcdef".into(),
                access_token: "TEST_ONLY_NEVER_LAUNCHED".into(),
                client_id: "TEST_ONLY".into(),
                xuid: "0".into(),
            };
            let (plan, major) = build_plan(
                &root,
                "fabric-smoke",
                "1.21.1",
                "fabric",
                report.loader_version.as_deref(),
                4096,
                true,
                &fixture,
            )
            .unwrap();
            assert_eq!(major, 21);
            assert!(plan
                .args
                .iter()
                .any(|a| a == "net.fabricmc.loader.impl.launch.knot.KnotClient"));
            assert!(plan.args.iter().any(|a| a == "--fullscreen"));
            assert!(!plan.args.iter().any(|a| a.contains("${")));
            if cfg!(target_os = "macos") {
                assert!(plan.args.iter().any(|a| a == "-XstartOnFirstThread"));
            }
            assert!(build_plan(
                &root,
                "fabric-smoke",
                "1.21.1",
                "fabric",
                Some("0.0.0"),
                4096,
                false,
                &fixture
            )
            .is_err());
            let loader_path = root.join("libraries").join(
                crate::fabric::maven_path(&format!(
                    "net.fabricmc:fabric-loader:{}",
                    report.loader_version.as_deref().unwrap()
                ))
                .unwrap(),
            );
            fs::write(&loader_path, b"corrupt test library").unwrap();
            assert!(build_plan(
                &root,
                "fabric-smoke",
                "1.21.1",
                "fabric",
                report.loader_version.as_deref(),
                4096,
                false,
                &fixture
            )
            .is_err());
            let repaired = mc::install_at(
                &root,
                "fabric-smoke",
                "1.21.1",
                "fabric",
                report.loader_version.as_deref(),
                Arc::new(|_| {}),
            )
            .await
            .unwrap();
            assert_eq!(repaired.loader_version, report.loader_version);
            build_plan(
                &root,
                "fabric-smoke",
                "1.21.1",
                "fabric",
                repaired.loader_version.as_deref(),
                4096,
                false,
                &fixture,
            )
            .unwrap();
            eprintln!(
                "FABRIC_REPORT {}",
                serde_json::to_string(&repaired).unwrap()
            );
            eprintln!("JAVA_REPORT {}", serde_json::to_string(&java).unwrap());
        });
    }

    #[test]
    #[ignore = "Requires the two real installation tests; verifies the combined modded launch plan without starting a game."]
    fn real_modded_launch_plan() {
        let root = PathBuf::from("/private/tmp/syncmc-fabric-smoke");
        let report: mc::InstallationReport =
            serde_json::from_value(read_json(&root, &mc::report_path("fabric-smoke")).unwrap())
                .unwrap();
        let lock = read_json(&root, Path::new("instances/fabric-smoke/content-lock.json")).unwrap();
        let contents: Vec<content::ContentSelection> =
            serde_json::from_value(lock["contents"].clone()).unwrap();
        assert!(contents.iter().any(|c| c.dependency));
        content::verify_installation(&root, "fabric-smoke", "1.21.1", "fabric", &contents).unwrap();
        let fixture = auth::MinecraftSession {
            name: "PlanTest".into(),
            uuid: "1234567890abcdef1234567890abcdef".into(),
            access_token: "TEST_ONLY_NEVER_LAUNCHED".into(),
            client_id: "TEST_ONLY".into(),
            xuid: "0".into(),
        };
        let (plan, _) = build_plan(
            &root,
            "fabric-smoke",
            "1.21.1",
            "fabric",
            report.loader_version.as_deref(),
            4096,
            false,
            &fixture,
        )
        .unwrap();
        assert!(plan
            .args
            .iter()
            .any(|a| a == "net.fabricmc.loader.impl.launch.knot.KnotClient"));
        let summary = json!({"minecraft":"1.21.1","loader":report.loader_version,"contentFiles":contents.len(),"requiredDependency":true,"checks":{"contentHashes":"passed","classpath":"passed","assets":"passed","natives":"passed","arguments":"passed"},"gameStarted":false,"authenticatedLoginTested":false});
        fs::write(
            root.join("modded-launch-plan-results.json"),
            serde_json::to_vec_pretty(&summary).unwrap(),
        )
        .unwrap();
        eprintln!("MODDED_LAUNCH_PLAN {summary}");
    }
}

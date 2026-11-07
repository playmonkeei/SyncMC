mod auth;
mod backup;
mod content;
mod curseforge;
mod fabric;
mod game_log;
mod launch;
mod managed_java;
mod minecraft;
mod modrinth;
mod profile_export;
mod runtime;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    use tauri::Emitter;
    tauri::Builder::default()
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if launch::ensure_stopped().is_err() {
                    api.prevent_close();
                    let _ = window.emit("minecraft-close-blocked", "Minecraft läuft noch. Bitte im Spiel speichern und das Spiel beenden, bevor du SyncMC schließt.");
                }
            }
        })
        .setup(|_app| {
            #[cfg(debug_assertions)]
            {
                use tauri::Manager;
                if let Some(window) = _app.get_webview_window("main") {
                    eprintln!(
                        "SyncMC native window ready: visible={}",
                        window.is_visible().unwrap_or(false)
                    );
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            runtime::runtime_info,
            runtime::validate_java,
            managed_java::install_java_runtime,
            fabric::get_fabric_loaders,
            auth::begin_microsoft_login,
            auth::poll_microsoft_login,
            auth::cancel_microsoft_login,
            auth::get_minecraft_account,
            auth::logout_minecraft_account,
            auth::open_microsoft_login,
            auth::open_microsoft_registration,
            content::install_instance_content,
            curseforge::curseforge_status,
            curseforge::set_curseforge_key,
            curseforge::clear_curseforge_key,
            curseforge::search_curseforge,
            curseforge::get_curseforge_versions,
            curseforge::open_curseforge_application,
            backup::create_instance_backup,
            backup::list_instance_backups,
            backup::restore_instance_backup,
            profile_export::save_profile_export,
            profile_export::open_profile_exports,
            launch::launch_minecraft,
            launch::game_status,
            launch::get_game_logs,
            launch::stop_minecraft,
            modrinth::search_modrinth,
            modrinth::get_modrinth_project_links,
            modrinth::get_modrinth_versions,
            minecraft::get_minecraft_versions,
            minecraft::install_minecraft,
            minecraft::get_minecraft_installation
        ])
        .build(tauri::generate_context!())
        .expect("SyncMC could not start its desktop runtime")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if launch::ensure_stopped().is_err() {
                    api.prevent_exit();
                    let _ = app.emit("minecraft-close-blocked", "Minecraft läuft noch. Bitte im Spiel speichern und das Spiel beenden, bevor du SyncMC schließt.");
                }
            }
        });
}

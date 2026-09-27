pub mod config;

use std::sync::Arc;
use tauri::Manager;

#[tauri::command]
fn start_tour() -> Result<(), String> { Ok(()) }
#[tauri::command]
fn pause_tour() -> Result<(), String> { Ok(()) }
#[tauri::command]
fn resume_tour() -> Result<(), String> { Ok(()) }
#[tauri::command]
fn next_tile() -> Result<(), String> { Ok(()) }
#[tauri::command]
fn prev_tile() -> Result<(), String> { Ok(()) }
#[tauri::command]
fn toggle_fullscreen() -> Result<(), String> { Ok(()) }
#[tauri::command]
fn minimize_current() -> Result<(), String> { Ok(()) }
#[tauri::command]
fn get_tour_status() -> Result<(), String> { Ok(()) }

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let app_data_dir = app
                .path()
                .app_config_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("config"));

            let exe_path = std::env::current_exe().ok();
            let meta = config::resolve_configuration(None, exe_path.as_deref(), &app_data_dir);
            let state = Arc::new(config::AppConfigState::new(meta, app_data_dir));
            app.manage(state);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            config::get_config,
            config::save_config,
            config::export_config,
            start_tour,
            pause_tour,
            resume_tour,
            next_tile,
            prev_tile,
            toggle_fullscreen,
            minimize_current,
            get_tour_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

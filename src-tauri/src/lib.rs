pub mod config;

use std::sync::Arc;
use tauri::Manager;


#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let app_data_dir = app
                .path()
                .app_config_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("config"));

            let cli_config_path = config::parse_cli_config_arg(std::env::args());
            let exe_path = std::env::current_exe().ok();
            let meta = config::resolve_configuration(
                cli_config_path.as_deref(),
                exe_path.as_deref(),
                &app_data_dir,
            );
            let state = Arc::new(config::AppConfigState::new(meta, app_data_dir));
            app.manage(state);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            config::get_config,
            config::save_config,
            config::export_config,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

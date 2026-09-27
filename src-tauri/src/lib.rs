pub mod config;
pub mod layout;
pub mod proxy;
pub mod tour;
pub mod webview_manager;

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
            )
            .map_err(|e| format!("Failed to initialize kiosk configuration: {}", e))?;

            let cfg = meta.config.clone();
            let config_state = Arc::new(config::AppConfigState::new(meta, app_data_dir));
            app.manage(config_state);

            // 1. Start loopback proxy if adblock DNS is active
            let proxy_port = if cfg.network_dns.adblock_dns_enabled {
                let p = tauri::async_runtime::block_on(async {
                    proxy::start_adguard_proxy(cfg.network_dns.clone()).await
                })
                .map_err(|e| format!("Failed to start AdGuard proxy: {}", e))?;
                Some(p)
            } else {
                None
            };

            // 2. Compute full-bleed layout for main coordinator window
            let window = app.get_window("main").expect("Window 'main' must exist");
            let (win_width, win_height) = {
                let size = window.inner_size().unwrap_or(tauri::PhysicalSize::new(1920, 1080));
                let scale = window.scale_factor().unwrap_or(1.0);
                (size.width as f64 / scale, size.height as f64 / scale)
            };

            let (rects, _) = layout::calculate_grid_layout(
                win_width,
                win_height,
                cfg.endpoints.len(),
                cfg.layout.target_tile_aspect_ratio,
                cfg.layout.rows.as_deref(),
                cfg.window.reserved_header_height_px,
                cfg.layout.padding_px,
                cfg.layout.gap_px,
            );

            // 3. Spawn full-resident native child webviews (M = N)
            let (page_load_tx, page_load_rx) = tokio::sync::mpsc::unbounded_channel();
            let tiles = webview_manager::spawn_resident_webviews(
                &window,
                &cfg.endpoints,
                &rects,
                proxy_port,
                page_load_tx,
            )?;

            // 4. Initialize TourController and start background tour cycle
            let tour_controller = tour::TourController::new(
                app.handle().clone(),
                cfg.timing.clone(),
                cfg.window.reserved_header_height_px,
                win_width,
                win_height,
                tiles,
            );
            app.manage(tour_controller.clone());

            tour::start_tour_loop(tour_controller, page_load_rx, cfg.tour.auto_start);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            config::get_config,
            config::save_config,
            config::export_config,
            tour::start_tour,
            tour::pause_tour,
            tour::resume_tour,
            tour::next_tile,
            tour::prev_tile,
            tour::toggle_fullscreen,
            tour::minimize_current,
            tour::get_tour_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

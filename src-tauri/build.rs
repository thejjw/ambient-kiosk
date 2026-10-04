fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "minimize_app",
            "request_close_app",
            "resolve_close_app",
            "get_close_pending",
            "get_config",
            "save_config",
            "export_config",
            "set_settings_open",
            "start_tour",
            "pause_tour",
            "resume_tour",
            "next_tile",
            "prev_tile",
            "toggle_fullscreen",
            "minimize_current",
            "get_tour_status",
            "get_diagnostics",
            "report_frontend_error",
        ]),
    ))
    .expect("failed to run tauri build");
}

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(
            tauri_build::AppManifest::new().commands(&[
                "get_config",
                "save_config",
                "export_config",
                "start_tour",
                "pause_tour",
                "resume_tour",
                "next_tile",
                "prev_tile",
                "toggle_fullscreen",
                "minimize_current",
                "get_tour_status",
            ]),
        ),
    )
    .expect("failed to run tauri build");
}

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(
            tauri_build::AppManifest::new().commands(&[
                "get_config",
                "save_config",
                "export_config",
            ]),
        ),
    )
    .expect("failed to run tauri build");
}

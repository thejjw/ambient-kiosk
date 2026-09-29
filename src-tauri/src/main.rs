#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(all(windows, not(debug_assertions)))]
fn configure_portable_runtime() {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(hwnd: isize, text: *const u16, caption: *const u16, kind: u32) -> i32;
    }

    let runtime = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join("runtime")));
    if let Some(path) = runtime.as_ref().filter(|path| path.join("msedgewebview2.exe").is_file()) {
        std::env::set_var("WEBVIEW2_BROWSER_EXECUTABLE_FOLDER", path);
        return;
    }

    let message: Vec<u16> = std::ffi::OsStr::new(
        "The bundled WebView2 runtime is missing. Extract the complete Ambient Kiosk ZIP and launch it from a local folder.",
    )
    .encode_wide()
    .chain(Some(0))
    .collect();
    let caption: Vec<u16> = std::ffi::OsStr::new("Ambient Kiosk startup error")
        .encode_wide()
        .chain(Some(0))
        .collect();
    unsafe { MessageBoxW(0, message.as_ptr(), caption.as_ptr(), 0x10) };
    std::process::exit(1);
}

fn main() {
    #[cfg(all(windows, not(debug_assertions)))]
    configure_portable_runtime();

    ambient_kiosk_lib::run();
}

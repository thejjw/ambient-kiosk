#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    ambient_kiosk_lib::run();
}

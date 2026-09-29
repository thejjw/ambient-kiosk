pub mod config;
pub mod layout;
pub mod proxy;
pub mod tour;
pub mod webview_manager;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{Emitter, Manager, PhysicalSize};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

fn get_kiosk_shortcuts() -> [Shortcut; 6] {
    [
        Shortcut::new(Some(Modifiers::empty()), Code::Space),
        Shortcut::new(Some(Modifiers::empty()), Code::ArrowRight),
        Shortcut::new(Some(Modifiers::empty()), Code::ArrowLeft),
        Shortcut::new(Some(Modifiers::empty()), Code::F11),
        Shortcut::new(Some(Modifiers::empty()), Code::Escape),
        Shortcut::new(Some(Modifiers::empty()), Code::KeyH),
    ]
}

async fn unregister_kiosk_shortcuts_bounded(app: &tauri::AppHandle) -> bool {
    for attempt in 1..=3 {
        if app.global_shortcut().unregister_all().is_ok() {
            return true;
        }
        for s in get_kiosk_shortcuts() {
            let _ = app.global_shortcut().unregister(s);
        }
        tokio::time::sleep(std::time::Duration::from_millis(50 * attempt)).await;
    }
    eprintln!("[Shortcuts] CRITICAL: unregister_kiosk_shortcuts failed after 3 bounded retries");
    false
}
struct FocusShortcutManager {
    main_focused: bool,
    hud_focused: bool,
}

struct HudInteractionState {
    settings_open: AtomicBool,
    shortcut_tx: tokio::sync::mpsc::UnboundedSender<bool>,
}

// Keep the overlay in the main window's physical client coordinates at every DPI.
fn align_hud(app: &tauri::AppHandle, settings_open: bool) -> Result<(), String> {
    let main = app.get_window("main").ok_or("Main window is unavailable")?;
    let hud = app
        .get_window("hud-overlay")
        .ok_or("HUD window is unavailable")?;
    let origin = main.inner_position().map_err(|e| e.to_string())?;
    let size = main.inner_size().map_err(|e| e.to_string())?;
    let scale = main.scale_factor().map_err(|e| e.to_string())?;
    let height = if settings_open {
        size.height.min((900.0 * scale).round() as u32)
    } else {
        (48.0 * scale).round() as u32
    };
    hud.set_position(origin).map_err(|e| e.to_string())?;
    hud.set_size(PhysicalSize::new(size.width, height.max(1)))
        .map_err(|e| e.to_string())
}

/// Change drawer interaction only from the trusted HUD webview.
#[tauri::command]
fn set_settings_open(
    webview_window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<HudInteractionState>>,
    open: bool,
) -> Result<(), String> {
    if webview_window.label() != "hud-overlay" {
        return Err("Only the HUD can change settings state".into());
    }
    align_hud(&app, open)?;
    app.get_window("hud-overlay")
        .ok_or("HUD window is unavailable")?
        .set_ignore_cursor_events(false)
        .map_err(|e| e.to_string())?;
    state.settings_open.store(open, Ordering::SeqCst);
    let focused = app
        .get_window("main")
        .and_then(|w| w.is_focused().ok())
        .unwrap_or(false)
        || webview_window.is_focused().unwrap_or(false);
    let _ = state.shortcut_tx.send(!open && focused);
    Ok(())
}

impl FocusShortcutManager {
    fn new() -> Self {
        Self {
            main_focused: false,
            hud_focused: false,
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let focus_state = Arc::new(parking_lot::Mutex::new(FocusShortcutManager::new()));
    let focus_state_cb = focus_state.clone();
    let focus_gen = Arc::new(std::sync::atomic::AtomicU64::new(1));
    let focus_gen_cb = focus_gen.clone();

    let (shortcut_tx, mut shortcut_rx) = tokio::sync::mpsc::unbounded_channel::<bool>();
    let shortcut_tx_cb = shortcut_tx.clone();
    let hud_state = Arc::new(HudInteractionState {
        settings_open: AtomicBool::new(false),
        shortcut_tx: shortcut_tx.clone(),
    });
    let hud_state_cb = hud_state.clone();

    tauri::Builder::default()
        .on_window_event(move |window, event| {
            let label = window.label().to_string();
            let app_handle = window.app_handle().clone();

            match event {
                tauri::WindowEvent::Focused(focused) => {
                    {
                        let mut state = focus_state_cb.lock();
                        if label == "main" {
                            state.main_focused = *focused;
                        } else if label == "hud-overlay" {
                            state.hud_focused = *focused;
                        }
                    }

                    let current_focus_gen = focus_gen_cb.fetch_add(1, Ordering::SeqCst) + 1;

                    if *focused {
                        // A minimized main window must never leave the HUD above other apps.
                        let minimized = app_handle
                            .get_window("main")
                            .and_then(|w| w.is_minimized().ok())
                            .unwrap_or(false);
                        if let Some(hud) = app_handle.get_window("hud-overlay") {
                            if minimized {
                                let _ = hud.hide();
                            } else {
                                let _ = hud.show();
                            }
                        }
                        let _ = shortcut_tx_cb.send(
                            !minimized && !hud_state_cb.settings_open.load(Ordering::SeqCst),
                        );
                    } else {
                        // Focus lost: debounce 150ms and re-query OS focus before requesting unregister
                        let app_handle_delayed = app_handle.clone();
                        let focus_state_delayed = focus_state_cb.clone();
                        let focus_gen_delayed = focus_gen_cb.clone();
                        let shortcut_tx_delayed = shortcut_tx_cb.clone();

                        tauri::async_runtime::spawn(async move {
                            tokio::time::sleep(std::time::Duration::from_millis(150)).await;

                            // Invalidate if newer focus event arrived during the 150ms window
                            if focus_gen_delayed.load(Ordering::SeqCst) != current_focus_gen {
                                return;
                            }

                            // Directly re-query OS focus state from both windows
                            let main_focused = app_handle_delayed
                                .get_window("main")
                                .and_then(|w| w.is_focused().ok())
                                .unwrap_or(false);
                            let hud_focused = app_handle_delayed
                                .get_window("hud-overlay")
                                .and_then(|w| w.is_focused().ok())
                                .unwrap_or(false);

                            if !main_focused && !hud_focused {
                                {
                                    let mut st = focus_state_delayed.lock();
                                    st.main_focused = false;
                                    st.hud_focused = false;
                                }

                                if let Some(hud) = app_handle_delayed.get_window("hud-overlay") {
                                    let _ = hud.hide();
                                }
                                let _ = shortcut_tx_delayed.send(false);
                            }
                        });
                    }
                }
                tauri::WindowEvent::Moved(_) if label == "main" => {
                    let _ = align_hud(
                        &app_handle,
                        hud_state_cb.settings_open.load(Ordering::SeqCst),
                    );
                }
                tauri::WindowEvent::Resized(size) if label == "main" => {
                    let scale = window.scale_factor().unwrap_or(1.0);
                    let logical_w = size.width as f64 / scale;
                    let logical_h = size.height as f64 / scale;

                    let minimized = window.is_minimized().unwrap_or(false);
                    if let Some(hud) = app_handle.get_window("hud-overlay") {
                        if minimized || size.width == 0 || size.height == 0 {
                            let _ = hud.hide();
                        } else {
                            let _ = align_hud(
                                &app_handle,
                                hud_state_cb.settings_open.load(Ordering::SeqCst),
                            );
                            if window.is_focused().unwrap_or(false)
                                || hud.is_focused().unwrap_or(false)
                            {
                                let _ = hud.show();
                            }
                        }
                    }

                    if let Some(ctrl) = app_handle.try_state::<Arc<tour::TourController>>() {
                        ctrl.inner().handle_window_resize(logical_w, logical_h);
                    }
                }
                tauri::WindowEvent::ScaleFactorChanged { .. } if label == "main" => {
                    let _ = align_hud(
                        &app_handle,
                        hud_state_cb.settings_open.load(Ordering::SeqCst),
                    );
                    if let (Ok(size), Ok(scale), Some(ctrl)) = (
                        window.inner_size(),
                        window.scale_factor(),
                        app_handle.try_state::<Arc<tour::TourController>>(),
                    ) {
                        ctrl.inner().handle_window_resize(
                            size.width as f64 / scale,
                            size.height as f64 / scale,
                        );
                    }
                }
                tauri::WindowEvent::CloseRequested { .. } if label == "main" => {
                    let _ = shortcut_tx_cb.send(false);
                    if let Some(hud) = app_handle.get_window("hud-overlay") {
                        let _ = hud.close();
                    }
                }
                tauri::WindowEvent::Destroyed if label == "main" => {
                    app_handle.exit(0);
                }
                _ => {}
            }
        })
        .setup(move |app| {
            app.manage(hud_state.clone());
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
            let _ = window.set_fullscreen(cfg.window.fullscreen);
            let _ = window.set_decorations(cfg.window.decorations);
            if let Some((r, g, b, a)) = config::parse_hex_color(&cfg.window.background_color) {
                let _ = window.set_background_color(Some(tauri::window::Color(r, g, b, a)));
            }
            let (win_width, win_height) = {
                let size = window
                    .inner_size()
                    .unwrap_or(tauri::PhysicalSize::new(1920, 1080));
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
                cfg.layout.clone(),
                cfg.window.reserved_header_height_px,
                win_width,
                win_height,
                tiles,
            );
            app.manage(tour_controller.clone());

            // 5. Setup global shortcut plugin with controller integration
            let ctrl_for_shortcuts = tour_controller.clone();
            let app_for_shortcuts = app.handle().clone();
            let hud_state_for_shortcuts = hud_state.clone();
            let (hud_cmd_tx, mut hud_cmd_rx) = tokio::sync::mpsc::unbounded_channel::<bool>();
            let hud_cmd_tx_sc = hud_cmd_tx.clone();
            let shortcut_plugin = tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |_app, shortcut, event| {
                    if event.state() == ShortcutState::Pressed
                        && !hud_state_for_shortcuts.settings_open.load(Ordering::SeqCst)
                    {
                        let space_sc = Shortcut::new(Some(Modifiers::empty()), Code::Space);
                        let right_sc = Shortcut::new(Some(Modifiers::empty()), Code::ArrowRight);
                        let left_sc = Shortcut::new(Some(Modifiers::empty()), Code::ArrowLeft);
                        let f11_sc = Shortcut::new(Some(Modifiers::empty()), Code::F11);
                        let esc_sc = Shortcut::new(Some(Modifiers::empty()), Code::Escape);
                        let h_sc = Shortcut::new(Some(Modifiers::empty()), Code::KeyH);

                        if shortcut == &space_sc {
                            let is_paused = ctrl_for_shortcuts.machine.state.lock().is_paused;
                            if is_paused {
                                ctrl_for_shortcuts.resume();
                            } else {
                                ctrl_for_shortcuts.pause();
                            }
                        } else if shortcut == &right_sc {
                            ctrl_for_shortcuts.next();
                        } else if shortcut == &left_sc {
                            ctrl_for_shortcuts.prev();
                        } else if shortcut == &f11_sc {
                            let _ = tour::toggle_fullscreen(app_for_shortcuts.clone());
                        } else if shortcut == &esc_sc {
                            ctrl_for_shortcuts.minimize_current();
                        } else if shortcut == &h_sc {
                            let _ = hud_cmd_tx_sc.send(true);
                        }
                    }
                })
                .build();

            app.handle().plugin(shortcut_plugin)?;

            // 6. Spawn sequential shortcut reconciler actor
            let app_handle_for_actor = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut currently_registered = false;

                while let Some(mut desired) = shortcut_rx.recv().await {
                    while let Ok(next) = shortcut_rx.try_recv() {
                        desired = next;
                    }

                    if desired && !currently_registered {
                        let mut reg_failed = false;
                        for s in get_kiosk_shortcuts() {
                            if let Err(e) = app_handle_for_actor.global_shortcut().register(s) {
                                eprintln!("[Shortcuts] Failed to register shortcut {:?}: {}", s, e);
                                reg_failed = true;
                                break;
                            }
                        }
                        if reg_failed {
                            let cleaned =
                                unregister_kiosk_shortcuts_bounded(&app_handle_for_actor).await;
                            currently_registered = !cleaned;
                        } else {
                            currently_registered = true;
                        }
                    } else if !desired
                        && currently_registered
                        && unregister_kiosk_shortcuts_bounded(&app_handle_for_actor).await
                    {
                        currently_registered = false;
                    }
                }
            });

            // Configure initial click-through state and align with the client area.
            if let Some(hud) = app.get_window("hud-overlay") {
                let _ = hud.set_ignore_cursor_events(true);
                align_hud(&app.handle().clone(), false)?;
            }

            // Seed initial shortcut desired focus state safely (default false for safety)
            let main_focused = window.is_focused().unwrap_or(false);
            let hud_focused = app
                .get_window("hud-overlay")
                .and_then(|h| h.is_focused().ok())
                .unwrap_or(false);
            let initial_focus = main_focused || hud_focused;
            let _ = shortcut_tx.send(initial_focus);
            // 7. Native cursor tracking poller owning interactive state and leave_timer
            let cursor_app = app.handle().clone();
            let hud_state_for_cursor = hud_state.clone();
            tauri::async_runtime::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_millis(50));
                let mut leave_timer = std::time::Instant::now();
                let mut is_interactive = false;

                loop {
                    interval.tick().await;

                    let main_win = match cursor_app.get_window("main") {
                        Some(w) => w,
                        None => continue,
                    };
                    let hud_win = match cursor_app.get_window("hud-overlay") {
                        Some(w) => w,
                        None => continue,
                    };

                    if hud_state_for_cursor.settings_open.load(Ordering::SeqCst) {
                        if !is_interactive {
                            let _ = hud_win.set_ignore_cursor_events(false);
                            is_interactive = true;
                        }
                        continue;
                    }

                    // Handle external toggle commands (e.g. from H hotkey)
                    while hud_cmd_rx.try_recv().is_ok() {
                        is_interactive = !is_interactive;
                        let _ = hud_win.set_ignore_cursor_events(!is_interactive);
                        let _ = cursor_app.emit_to("hud-overlay", "hud-visibility", is_interactive);
                        if is_interactive {
                            leave_timer = std::time::Instant::now();
                        }
                    }

                    if let Ok(cursor_pos) = main_win.cursor_position() {
                        if let Ok(win_pos) = main_win.inner_position() {
                            if let Ok(win_size) = main_win.inner_size() {
                                let scale = main_win.scale_factor().unwrap_or(1.0);
                                let cx = cursor_pos.x;
                                let cy = cursor_pos.y;
                                let wx = win_pos.x as f64;
                                let wy = win_pos.y as f64;
                                let ww = win_size.width as f64;

                                let in_top_edge = cx >= wx
                                    && cx <= wx + ww
                                    && cy >= wy
                                    && cy <= wy + (20.0 * scale);
                                let in_hud_band = cx >= wx
                                    && cx <= wx + ww
                                    && cy >= wy
                                    && cy <= wy + (52.0 * scale);

                                if in_top_edge {
                                    if !is_interactive {
                                        let _ = cursor_app.emit_to(
                                            "hud-overlay",
                                            "hud-visibility",
                                            true,
                                        );
                                        let _ = hud_win.set_ignore_cursor_events(false);
                                        is_interactive = true;
                                    }
                                    leave_timer = std::time::Instant::now();
                                } else if in_hud_band && is_interactive {
                                    leave_timer = std::time::Instant::now();
                                } else if is_interactive
                                    && leave_timer.elapsed()
                                        >= std::time::Duration::from_millis(1500)
                                {
                                    let _ =
                                        cursor_app.emit_to("hud-overlay", "hud-visibility", false);
                                    let _ = hud_win.set_ignore_cursor_events(true);
                                    is_interactive = false;
                                }
                            }
                        }
                    }
                }
            });

            tour::start_tour_loop(tour_controller, page_load_rx, cfg.tour.auto_start);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            config::get_config,
            config::save_config,
            config::export_config,
            set_settings_open,
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

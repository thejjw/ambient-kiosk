use crate::config::EndpointItem;
use crate::diagnostics::DiagnosticsState;
use crate::layout::LogicalRect;
use parking_lot::Mutex;
use std::sync::Arc;
#[cfg(target_os = "windows")]
use std::time::{SystemTime, UNIX_EPOCH};
#[cfg(target_os = "windows")]
use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt, sync::OnceLock};
#[cfg(target_os = "windows")]
use tauri::Manager;
use tauri::{
    webview::{NewWindowResponse, PageLoadEvent, WebviewBuilder},
    LogicalPosition, LogicalSize, Webview, WebviewUrl, Window,
};
use tokio::sync::mpsc::UnboundedSender;
use url::Url;

/// Tracks lifecycle state of an individual tile reload to establish strict event provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileLoadState {
    Idle,
    Navigating,
    ReloadRequested { generation: u64 },
    Loading { generation: u64 },
}

/// Page-load observation with a best-effort refresh generation association.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageLoadObservation {
    Started { generation: Option<u64> },
    Finished { generation: Option<u64> },
    Ignored,
}

/// Thread-safe coordinator establishing causal event correlation for webview reloads.
pub struct TileLoadCoordinator {
    pub state: Mutex<TileLoadState>,
}
impl Default for TileLoadCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl TileLoadCoordinator {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(TileLoadState::Idle),
        }
    }

    /// Marks that a reload was requested for the given generation token.
    pub fn request_reload(&self, generation: u64) {
        let mut s = self.state.lock();
        *s = TileLoadState::ReloadRequested { generation };
    }

    /// Retires callbacks for one generation while leaving newer work intact.
    pub fn cancel_reload(&self, generation: u64) {
        let mut state = self.state.lock();
        if matches!(*state, TileLoadState::ReloadRequested { generation: current } | TileLoadState::Loading { generation: current } if current == generation)
        {
            *state = TileLoadState::Idle;
        }
    }

    /// Correlates the callback sequence with a refresh when one is pending.
    pub fn handle_page_load(&self, event: PageLoadEvent) -> PageLoadObservation {
        let mut s = self.state.lock();
        match (*s, event) {
            (TileLoadState::ReloadRequested { generation }, PageLoadEvent::Started) => {
                *s = TileLoadState::Loading { generation };
                PageLoadObservation::Started {
                    generation: Some(generation),
                }
            }
            (TileLoadState::Loading { generation }, PageLoadEvent::Finished) => {
                *s = TileLoadState::Idle;
                PageLoadObservation::Finished {
                    generation: Some(generation),
                }
            }
            (TileLoadState::Navigating, PageLoadEvent::Finished) => {
                *s = TileLoadState::Idle;
                PageLoadObservation::Finished { generation: None }
            }
            (TileLoadState::Idle, PageLoadEvent::Started) => {
                *s = TileLoadState::Navigating;
                PageLoadObservation::Started { generation: None }
            }
            (TileLoadState::Loading { generation }, PageLoadEvent::Started) => {
                PageLoadObservation::Started {
                    generation: Some(generation),
                }
            }
            (TileLoadState::Navigating, PageLoadEvent::Started) => {
                PageLoadObservation::Started { generation: None }
            }
            // Ignore a finish arriving before the requested refresh has started.
            (TileLoadState::ReloadRequested { .. }, PageLoadEvent::Finished)
            | (TileLoadState::Idle, PageLoadEvent::Finished) => PageLoadObservation::Ignored,
        }
    }
}

/// Represents an active resident child webview tile.
pub struct ManagedWebviewTile {
    pub index: usize,
    pub id: String,
    pub title: String,
    pub url: String,
    pub webview: Webview,
    pub resting_rect: parking_lot::Mutex<LogicalRect>,
    pub coordinator: Arc<TileLoadCoordinator>,
}

impl ManagedWebviewTile {
    /// Initiates a native reload paired with the specified generation token.
    pub fn trigger_reload(&self, generation: u64) -> Result<(), tauri::Error> {
        self.coordinator.request_reload(generation);
        self.webview.reload()
    }

    /// Retires callbacks for an attempt after its timeout, failure, or cancellation.
    pub fn cancel_reload(&self, generation: u64) {
        self.coordinator.cancel_reload(generation);
    }
}

/// Security validation: ensures webviews can only navigate to standard HTTP and HTTPS schemes.
/// Disallows file:, javascript:, data:, and custom IPC protocols.
pub fn is_allowed_navigation_scheme(url: &Url) -> bool {
    url.scheme() == "https" || url.scheme() == "http"
}

/// Creates one isolated WebView2 data directory for all guest tiles in this launch.
#[cfg(target_os = "windows")]
fn guest_data_directory(window: &Window) -> Result<std::path::PathBuf, String> {
    static SESSION_LOCK: OnceLock<std::fs::File> = OnceLock::new();
    let local_data = window
        .app_handle()
        .path()
        .app_local_data_dir()
        .map_err(|e| format!("Failed to locate local browser data directory: {e}"))?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("Failed to generate guest browser session ID: {e}"))?
        .as_nanos();
    let profiles = local_data.join("guest-profiles");
    std::fs::create_dir_all(&profiles)
        .map_err(|e| format!("Failed to create guest browser profile root: {e}"))?;
    cleanup_inactive_guest_profiles(&profiles);

    let directory = profiles.join(format!("session-{}-{timestamp}", std::process::id()));
    std::fs::create_dir_all(&directory)
        .map_err(|e| format!("Failed to create guest browser data directory: {e}"))?;
    let marker = OpenOptions::new()
        .write(true)
        .create_new(true)
        .share_mode(0)
        .open(directory.join("session.lock"))
        .map_err(|e| format!("Failed to lock guest browser data directory: {e}"))?;
    SESSION_LOCK
        .set(marker)
        .map_err(|_| "Guest browser session was initialized twice".to_string())?;
    Ok(directory)
}

/// Removes only prior application sessions whose exclusive lock is no longer held.
#[cfg(target_os = "windows")]
fn cleanup_inactive_guest_profiles(profiles: &std::path::Path) {
    let Ok(resolved_root) = std::fs::canonicalize(profiles) else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(profiles) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with("session-")
            || !entry
                .file_type()
                .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink())
        {
            continue;
        }
        let Ok(resolved_candidate) = std::fs::canonicalize(&path) else {
            continue;
        };
        if resolved_candidate.parent() != Some(resolved_root.as_path()) {
            continue;
        }
        let marker = path.join("session.lock");
        if !marker.is_file() {
            continue;
        }
        // A running instance holds this file exclusively. A stale session can be
        // removed after its owner exits and WebView2 releases its browser files.
        if OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(&marker)
            .is_ok()
        {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

/// Spawns resident child native webviews for all configured endpoints ($M = N$).
/// Each webview is securely quarantined with zero IPC permissions, constrained navigation,
/// and automatic popup rejection.
pub fn spawn_resident_webviews(
    window: &Window,
    endpoints: &[EndpointItem],
    rects: &[LogicalRect],
    proxy_port: Option<u16>,
    page_load_tx: UnboundedSender<(usize, PageLoadObservation)>,
    diagnostics: Arc<DiagnosticsState>,
) -> Result<Vec<ManagedWebviewTile>, String> {
    if endpoints.len() != rects.len() {
        return Err(format!(
            "Mismatch between endpoint count ({}) and computed layout slot count ({})",
            endpoints.len(),
            rects.len()
        ));
    }

    let proxy_url_parsed =
        proxy_port.and_then(|port| Url::parse(&format!("http://127.0.0.1:{}", port)).ok());

    #[cfg(target_os = "windows")]
    let guest_data_dir = guest_data_directory(window)?;

    let mut tiles = Vec::with_capacity(endpoints.len());

    for (idx, (ep, rect)) in endpoints.iter().zip(rects.iter()).enumerate() {
        let label = format!("guest-{}", idx);
        let parsed_url: Url = ep
            .url
            .parse()
            .map_err(|e| format!("Invalid URL for endpoint '{}': {}", ep.title, e))?;

        let mut builder = WebviewBuilder::new(&label, WebviewUrl::External(parsed_url));

        #[cfg(target_os = "windows")]
        {
            // WebView2 shares environment options by data directory. Keep all guests
            // together while separating their proxy setting from controller webviews.
            builder = builder.data_directory(guest_data_dir.clone());
        }

        if let Some(p_url) = &proxy_url_parsed {
            builder = builder.proxy_url(p_url.clone());
        }

        // Security Policy 1: Constrain navigation strictly to HTTP/HTTPS
        builder = builder.on_navigation(is_allowed_navigation_scheme);

        // Security Policy 2: Reject all popup windows from guest web content
        builder = builder.on_new_window(|_url, _features| NewWindowResponse::Deny);

        // The WebView callback has no request ID; generation association is best effort.
        let coordinator = Arc::new(TileLoadCoordinator::new());
        let coord_cb = coordinator.clone();
        let tx = page_load_tx.clone();
        let diagnostics_cb = diagnostics.clone();
        let endpoint_id_cb = ep.id.clone();

        builder = builder.on_page_load(move |_wv, payload| {
            let observation = coord_cb.handle_page_load(payload.event());
            if !matches!(observation, PageLoadObservation::Ignored) {
                if tx.send((idx, observation)).is_err() {
                    diagnostics_cb.event(
                        tracing::Level::WARN,
                        "webview",
                        "page_load_event_dropped",
                        serde_json::json!({ "endpoint_id": endpoint_id_cb, "tile_index": idx }),
                    );
                }
            }
        });

        let webview = match window.add_child(
            builder,
            LogicalPosition::new(rect.x, rect.y),
            LogicalSize::new(rect.width, rect.height),
        ) {
            Ok(webview) => webview,
            Err(error) => {
                diagnostics.event(
                    tracing::Level::ERROR,
                    "webview",
                    "guest_webview_create_failed",
                    serde_json::json!({ "endpoint_id": ep.id, "tile_index": idx }),
                );
                return Err(format!(
                    "Failed to attach child webview '{}': {}",
                    label, error
                ));
            }
        };

        diagnostics.event(
            tracing::Level::INFO,
            "webview",
            "guest_webview_created",
            serde_json::json!({ "endpoint_id": ep.id, "tile_index": idx }),
        );

        tiles.push(ManagedWebviewTile {
            index: idx,
            id: ep.id.clone(),
            title: ep.title.clone(),
            url: ep.url.clone(),
            webview,
            resting_rect: parking_lot::Mutex::new(*rect),
            coordinator,
        });
    }

    Ok(tiles)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "windows")]
    #[test]
    fn cleanup_preserves_active_and_unmarked_profiles() {
        let root = std::env::temp_dir().join(format!(
            "ambient-kiosk-profile-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let active = root.join("session-active");
        let stale = root.join("session-stale");
        let unmarked = root.join("session-unmarked");
        for path in [&active, &stale, &unmarked] {
            std::fs::create_dir_all(path).unwrap();
        }
        let active_lock = OpenOptions::new()
            .write(true)
            .create_new(true)
            .share_mode(0)
            .open(active.join("session.lock"))
            .unwrap();
        std::fs::write(stale.join("session.lock"), []).unwrap();

        cleanup_inactive_guest_profiles(&root);
        assert!(active.exists());
        assert!(!stale.exists());
        assert!(unmarked.exists());

        drop(active_lock);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn test_navigation_scheme_security() {
        assert!(is_allowed_navigation_scheme(
            &Url::parse("https://example.com/").unwrap()
        ));
        assert!(is_allowed_navigation_scheme(
            &Url::parse("http://example.com/").unwrap()
        ));
        assert!(!is_allowed_navigation_scheme(
            &Url::parse("file:///etc/passwd").unwrap()
        ));
        assert!(!is_allowed_navigation_scheme(
            &Url::parse("data:text/html,<html>").unwrap()
        ));
        assert!(!is_allowed_navigation_scheme(
            &Url::parse("javascript:alert(1)").unwrap()
        ));
        assert!(!is_allowed_navigation_scheme(
            &Url::parse("tauri://localhost").unwrap()
        ));
        assert!(!is_allowed_navigation_scheme(
            &Url::parse("custom://malicious").unwrap()
        ));
    }

    #[test]
    fn test_page_load_generation_tracking_and_wraparound_stale_rejection() {
        let coord = TileLoadCoordinator::new();

        // An unrelated page load is observed without being assigned a reload generation.
        assert_eq!(
            coord.handle_page_load(PageLoadEvent::Finished),
            PageLoadObservation::Ignored
        );
        assert_eq!(
            coord.handle_page_load(PageLoadEvent::Started),
            PageLoadObservation::Started { generation: None }
        );
        assert_eq!(
            coord.handle_page_load(PageLoadEvent::Finished),
            PageLoadObservation::Finished { generation: None }
        );

        // 2. Request reload for Generation 10
        coord.request_reload(10);

        // A stale Finished arriving before Started is discarded!
        assert_eq!(
            coord.handle_page_load(PageLoadEvent::Finished),
            PageLoadObservation::Ignored
        );

        // Started arrives -> enters Loading { generation: 10 }
        assert_eq!(
            coord.handle_page_load(PageLoadEvent::Started),
            PageLoadObservation::Started {
                generation: Some(10)
            }
        );

        // 3. Wraparound scenario: Generation 10 stalls and tour wraps around to request Generation 20
        coord.request_reload(20);

        // Late Finished event from Generation 10 arrives now!
        // Because state is ReloadRequested { generation: 20 }, G10's Finished is rejected!
        assert_eq!(
            coord.handle_page_load(PageLoadEvent::Finished),
            PageLoadObservation::Ignored
        );

        // New Started arrives for Generation 20 -> enters Loading { generation: 20 }
        assert_eq!(
            coord.handle_page_load(PageLoadEvent::Started),
            PageLoadObservation::Started {
                generation: Some(20)
            }
        );

        // Real Finished arrives for Generation 20 -> accepted and delivers generation 20!
        assert_eq!(
            coord.handle_page_load(PageLoadEvent::Finished),
            PageLoadObservation::Finished {
                generation: Some(20)
            }
        );

        // Subsequent spurious Finished is discarded
        assert_eq!(
            coord.handle_page_load(PageLoadEvent::Finished),
            PageLoadObservation::Ignored
        );
    }

    #[test]
    fn cancellation_retires_late_completion_for_that_generation() {
        let coord = TileLoadCoordinator::new();
        coord.request_reload(30);
        assert_eq!(
            coord.handle_page_load(PageLoadEvent::Started),
            PageLoadObservation::Started {
                generation: Some(30)
            }
        );
        coord.cancel_reload(30);
        assert_eq!(
            coord.handle_page_load(PageLoadEvent::Finished),
            PageLoadObservation::Ignored
        );
    }
}

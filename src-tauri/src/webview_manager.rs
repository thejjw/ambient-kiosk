use crate::config::EndpointItem;
use crate::layout::LogicalRect;
use parking_lot::Mutex;
use std::sync::Arc;
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
    ReloadRequested { generation: u64 },
    Loading { generation: u64 },
}

/// Thread-safe coordinator establishing causal event correlation for webview reloads.
pub struct TileLoadCoordinator {
    pub state: Mutex<TileLoadState>,
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

    /// Processes PageLoadEvent and returns Some(generation) only when Finished genuinely
    /// pairs with the in-flight reload generation. Discards stale or unrequested events.
    pub fn handle_page_load(&self, event: PageLoadEvent) -> Option<u64> {
        let mut s = self.state.lock();
        match (*s, event) {
            (TileLoadState::ReloadRequested { generation }, PageLoadEvent::Started) => {
                *s = TileLoadState::Loading { generation };
                None
            }
            (TileLoadState::Loading { generation }, PageLoadEvent::Finished) => {
                *s = TileLoadState::Idle;
                Some(generation)
            }
            // Stale Finished arriving while ReloadRequested (before new Started) is discarded!
            (TileLoadState::ReloadRequested { .. }, PageLoadEvent::Finished) => None,
            // Spontaneous or unrequested Finished is discarded
            (TileLoadState::Idle, _) => None,
            // Re-entrant Started during Loading retains current generation
            (TileLoadState::Loading { .. }, PageLoadEvent::Started) => None,
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
}

/// Security validation: ensures webviews can only navigate to standard HTTP and HTTPS schemes.
/// Disallows file:, javascript:, data:, and custom IPC protocols.
pub fn is_allowed_navigation_scheme(url: &Url) -> bool {
    url.scheme() == "https" || url.scheme() == "http"
}

/// Spawns resident child native webviews for all configured endpoints ($M = N$).
/// Each webview is securely quarantined with zero IPC permissions, constrained navigation,
/// and automatic popup rejection.
pub fn spawn_resident_webviews(
    window: &Window,
    endpoints: &[EndpointItem],
    rects: &[LogicalRect],
    proxy_port: Option<u16>,
    page_load_tx: UnboundedSender<(usize, u64)>,
) -> Result<Vec<ManagedWebviewTile>, String> {
    if endpoints.len() != rects.len() {
        return Err(format!(
            "Mismatch between endpoint count ({}) and computed layout slot count ({})",
            endpoints.len(),
            rects.len()
        ));
    }

    let proxy_url_parsed = proxy_port.and_then(|port| {
        Url::parse(&format!("http://127.0.0.1:{}", port)).ok()
    });

    let mut tiles = Vec::with_capacity(endpoints.len());

    for (idx, (ep, rect)) in endpoints.iter().zip(rects.iter()).enumerate() {
        let label = format!("guest-{}", idx);
        let parsed_url: Url = ep
            .url
            .parse()
            .map_err(|e| format!("Invalid URL for endpoint '{}': {}", ep.title, e))?;

        let mut builder = WebviewBuilder::new(&label, WebviewUrl::External(parsed_url));

        if let Some(p_url) = &proxy_url_parsed {
            builder = builder.proxy_url(p_url.clone());
        }

        // Security Policy 1: Constrain navigation strictly to HTTP/HTTPS
        builder = builder.on_navigation(|nav_url| is_allowed_navigation_scheme(nav_url));

        // Security Policy 2: Reject all popup windows from guest web content
        builder = builder.on_new_window(|_url, _features| NewWindowResponse::Deny);

        // Event relay for tour engine pre-refresh tracking with strict generation provenance
        let coordinator = Arc::new(TileLoadCoordinator::new());
        let coord_cb = coordinator.clone();
        let tx = page_load_tx.clone();

        builder = builder.on_page_load(move |_wv, payload| {
            if let Some(gen) = coord_cb.handle_page_load(payload.event()) {
                let _ = tx.send((idx, gen));
            }
        });

        let webview = window
            .add_child(
                builder,
                LogicalPosition::new(rect.x, rect.y),
                LogicalSize::new(rect.width, rect.height),
            )
            .map_err(|e| format!("Failed to attach child webview '{}': {}", label, e))?;

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

    #[test]
    fn test_navigation_scheme_security() {
        assert!(is_allowed_navigation_scheme(&Url::parse("https://example.com/").unwrap()));
        assert!(is_allowed_navigation_scheme(&Url::parse("http://example.com/").unwrap()));
        assert!(!is_allowed_navigation_scheme(&Url::parse("file:///etc/passwd").unwrap()));
        assert!(!is_allowed_navigation_scheme(&Url::parse("data:text/html,<html>").unwrap()));
        assert!(!is_allowed_navigation_scheme(&Url::parse("javascript:alert(1)").unwrap()));
        assert!(!is_allowed_navigation_scheme(&Url::parse("tauri://localhost").unwrap()));
        assert!(!is_allowed_navigation_scheme(&Url::parse("custom://malicious").unwrap()));
    }

    #[test]
    fn test_event_correlated_provenance_and_wraparound_stale_rejection() {
        let coord = TileLoadCoordinator::new();

        // 1. Spontaneous events while Idle are discarded
        assert_eq!(coord.handle_page_load(PageLoadEvent::Finished), None);
        assert_eq!(coord.handle_page_load(PageLoadEvent::Started), None);

        // 2. Request reload for Generation 10
        coord.request_reload(10);

        // A stale Finished arriving before Started is discarded!
        assert_eq!(coord.handle_page_load(PageLoadEvent::Finished), None);

        // Started arrives -> enters Loading { generation: 10 }
        assert_eq!(coord.handle_page_load(PageLoadEvent::Started), None);

        // 3. Wraparound scenario: Generation 10 stalls and tour wraps around to request Generation 20
        coord.request_reload(20);

        // Late Finished event from Generation 10 arrives now!
        // Because state is ReloadRequested { generation: 20 }, G10's Finished is rejected!
        assert_eq!(coord.handle_page_load(PageLoadEvent::Finished), None);

        // New Started arrives for Generation 20 -> enters Loading { generation: 20 }
        assert_eq!(coord.handle_page_load(PageLoadEvent::Started), None);

        // Real Finished arrives for Generation 20 -> accepted and delivers generation 20!
        assert_eq!(coord.handle_page_load(PageLoadEvent::Finished), Some(20));

        // Subsequent spurious Finished is discarded
        assert_eq!(coord.handle_page_load(PageLoadEvent::Finished), None);
    }
}

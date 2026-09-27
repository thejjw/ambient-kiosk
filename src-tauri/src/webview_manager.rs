use crate::config::EndpointItem;
use crate::layout::LogicalRect;
use tauri::{
    webview::{NewWindowResponse, PageLoadEvent, WebviewBuilder},
    LogicalPosition, LogicalSize, Webview, WebviewUrl, Window,
};
use tokio::sync::mpsc::UnboundedSender;
use url::Url;

/// Represents an active resident child webview tile.
pub struct ManagedWebviewTile {
    pub index: usize,
    pub id: String,
    pub title: String,
    pub url: String,
    pub webview: Webview,
    pub resting_rect: LogicalRect,
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
    page_load_tx: UnboundedSender<(usize, PageLoadEvent)>,
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

        // Event relay for tour engine pre-refresh tracking
        let tx = page_load_tx.clone();
        builder = builder.on_page_load(move |_wv, payload| {
            let _ = tx.send((idx, payload.event()));
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
            resting_rect: *rect,
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
}

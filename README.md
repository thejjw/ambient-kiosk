# Ambient Kiosk

Ambient Kiosk is an ambient workspace and idle-screen dashboard application built with **Tauri v2**. It organizes arbitrary web endpoints into an auto-tiling grid (e.g. $3 \times 2$) and runs an automated tour—elevating each site to full screen with a smooth transition, holding for a configured duration (e.g., 30 seconds), minimizing it back to its grid slot, and cycling through the list.

---

## The Framing Problem & Why Tauri v2?

Standard browser applications cannot embed arbitrary public websites (such as news outlets, financial dashboards, and tech aggregators) due to modern browser security mechanisms:
* **`X-Frame-Options: DENY` or `SAMEORIGIN`**
* **`Content-Security-Policy: frame-ancestors 'self'`**

If loaded in standard HTML `<iframe>` tags, external sites are blocked by the browser engine.

### How Ambient Kiosk Solves This
Instead of using `<iframe>` tags or stripping HTTP security headers via fragile reverse proxies, Ambient Kiosk leverages **Tauri v2 Native Multi-Webviews**:
1. **Top-Level Guest Browsing Contexts**: Each dashboard slot is instantiated as an independent child `Webview` (`WebviewBuilder`) attached to the host window.
2. **Zero Framing Constraints**: Because each child webview is an independent native top-level browser context (WKWebView on macOS, WebView2 on Windows), `X-Frame-Options` and `frame-ancestors` do not apply.
3. **Strict Security Isolation**:
   * **Zero IPC Capabilities**: Remote URLs are never mapped to Tauri command capabilities; untrusted guest scripts cannot call native Rust APIs or access the host filesystem.
   * **Navigation Restrictions (`on_navigation`)**: Prevents untrusted guest pages from navigating to unauthorized origins or protocols (`file://`, `tauri://`).
   * **Popup Suppression (`on_new_window`)**: Intercepts `window.open` and `<a target="_blank">` to prevent rogue window breakouts.

---

## Documentation

* **[Architecture Specification](docs/ARCHITECTURE.md)**: Deep dive into the desktop architecture comparison (Electron `WebContentsView` vs. Tauri v2 `Webview`), security threat model, auto-tiling grid geometry, and coordinate interpolation motion mechanics.
* **[Implementation Plan](docs/IMPLEMENTATION_PLAN.md)**: Six-phase engineering roadmap from project scaffolding to tour state machine, UI controls, and performance validation.

---

## Project Structure

```
ambient-kiosk/
├── docs/
│   ├── ARCHITECTURE.md          # Comprehensive architecture & security spec
│   └── IMPLEMENTATION_PLAN.md   # Step-by-step development roadmap
├── README.md                    # Project overview & quickstart
└── (src-tauri / src)            # Application codebase (scaffolded in Phase 1)
```

---

## Prerequisites

* **Rust & Cargo**: >= 1.78 (`rustc --version`)
* **Bun**: >= 1.0 (`bun --version`) or Node.js >= 20
* **OS Dependencies**:
  * macOS: Xcode Command Line Tools
  * Linux: `webkit2gtk-4.1`, `libssl-dev`, `libgtk-3-dev`
  * Windows: Microsoft Edge WebView2 runtime (preinstalled on Windows 10/11)

---

## License

Copyright (c) 2026 @thejjw

This software is provided 'as-is', without any express or implied warranty. In no event will the authors be held liable for any damages arising from the use of this software.

Permission is granted to anyone to use this software for any purpose, including commercial applications, and to alter it and redistribute it freely, subject to the following restrictions:

1. The origin of this software must not be misrepresented; you must not claim that you wrote the original software. If you use this software in a product, an acknowledgment (see the following) in the product documentation is required.

     Portions Copyright (c) 2026 @thejjw

2. Altered versions must be plainly marked as such, and must not be misrepresented as being the original software.

3. This notice may not be removed or altered from any distribution.

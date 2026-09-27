# Ambient Kiosk

Ambient Kiosk is an ambient workspace and idle-screen dashboard application built with **Tauri v2**. It organizes arbitrary web endpoints into an auto-tiling, full-bleed grid occupying 100% of the screen estate without scrollbars or clipping (scaling dynamically from $1 \times 1$ up to dense $M \times N$ layouts as endpoint count changes), and continuously alternates between:
1. **Multi-Website Grid View (Overview)**: All sites visible side-by-side in full-bleed layout for a configurable overview period (e.g., 20 seconds).
2. **Single-Website Maximized View (Deep Read)**: A single site expands smoothly to full window size, holds for a dedicated reading period (e.g., 30 seconds), minimizes back, and returns to the multi-site grid view before advancing to the next site.

### Default Presets (News & Markets)
1. **Biztoc** (`https://biztoc.com/`)
2. **Alltoc** (`https://alltoc.com/`)
3. **Biztoc Wire** (`https://biztoc.com/wire`)
4. **AP News Latest** (`https://apnews.com/hub/latest-news`)
5. **Finviz News** (`https://finviz.com/news`)

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
4. **Pipelined Pre-Refresh (Planned)**: Design initiates a native background reload (`Webview::reload()`) on the upcoming tile during minimization to pipeline content updates before expansion (readiness and fallback behavior subject to Phase 0 feasibility spike).
5. **Full-Resident Single-Window Grid**: Allocates native child webviews for all $N$ configured sites ($M = N$) so every endpoint is simultaneously visible in the grid without outer scrolling, guarded by a configurable safety ceiling (`max_resident_webviews: 12`).
6. **AdGuard DNS Adblocking (Planned / Spike)**: Proposed architecture defaults to routing child webview requests through an embedded loopback proxy resolving via AdGuard DNS-over-HTTPS (`https://dns.adguard-dns.com/dns-query`) or plain DNS fallback (`94.140.14.14:53`), with a user configurable option to opt into unproxied system DNS (macOS WKWebView proxy viability subject to Phase 0 spike).
7. **High Configurability & Shadowing Protection**:
   * Resolution Precedence: CLI/Env override (`--config`) > portable bundle-adjacent config (read-only) > writable OS AppData preferences > compiled defaults.
   * Source-Aware Safety: When a portable override is active, UI settings operate in read-only mode with "Export / Save As" to prevent silent shadowing; in-place saves write to AppData only when running without overrides.
---

## Documentation

* **[Architecture Specification](docs/ARCHITECTURE.md)**: Deep dive into the desktop architecture comparison (Electron `WebContentsView` vs. Tauri v2 `Webview`), security threat model, auto-tiling grid geometry, and coordinate interpolation motion mechanics.
* **[Implementation Plan](docs/IMPLEMENTATION_PLAN.md)**: Six-phase engineering roadmap from project scaffolding to tour state machine, UI controls, and performance validation.
* **[Manual Testing Guide](docs/MANUAL_TESTING.md)**: Runtime verification checklist for 5-feed layout, HUD overlay, hotkeys, settings, and focus management.

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

## Prerequisites & Cross-Platform Build Guide

* **Rust & Cargo**: >= 1.90 (`rustc --version`)
* **Bun**: >= 1.0 (`bun --version`) or Node.js >= 20
* **Official Reference**: Consult the [Tauri v2 Prerequisites Guide](https://v2.tauri.app/start/prerequisites/) for official distribution setup details.

### OS-Specific Build Requirements:

* **macOS**:
  * Xcode Command Line Tools: `xcode-select --install`
  * Runtime Engine: Native WKWebView (macOS 14+ required for native webview loopback proxying).

* **Linux (Ubuntu / Debian / Fedora / Arch)**:
  * Ubuntu / Debian:
    ```bash
    sudo apt update
    sudo apt install libwebkit2gtk-4.1-dev \
      build-essential \
      curl \
      wget \
      file \
      libxdo-dev \
      libssl-dev \
      libayatana-appindicator3-dev \
      librsvg2-dev
    ```
  * Fedora:
    ```bash
    sudo dnf install webkit2gtk4.1-devel \
      openssl-devel \
      curl \
      wget \
      file \
      libappindicator-gtk3-devel \
      librsvg2-devel
    ```
  * Arch Linux:
    ```bash
    sudo pacman -S webkit2gtk-4.1 \
      base-devel \
      curl \
      wget \
      file \
      openssl \
      libappindicator-gtk3 \
      librsvg
    ```
  * Runtime Engine: WebKitGTK 4.1.
* **Windows**:
  * Microsoft Visual Studio C++ Build Tools (MSVC) with Windows 10/11 SDK.
  * Microsoft Edge WebView2 runtime (pre-installed on Windows 10/11).
  * Runtime Engine: WebView2 (Chromium).

---

## Building & Running

### 1. Install Frontend Dependencies
```bash
bun install
```

### 2. Development Mode (with Live Reload)
```bash
bun run tauri dev
```

### 3. Production Release Build
```bash
bun run tauri build
```
---

## License

This project is licensed under the terms described in the [LICENSE](LICENSE) file.

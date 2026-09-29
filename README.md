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
2. **Zero Framing Constraints**: Because each child webview is an independent native top-level browser context (WKWebView on macOS, WebView2 on Windows, WebKitGTK on Linux), `X-Frame-Options` and `frame-ancestors` do not apply.
3. **Strict Security Isolation**:
   * **Zero IPC Capabilities**: Remote URLs are never mapped to Tauri command capabilities; untrusted guest scripts cannot call native Rust APIs or access the host filesystem.
   * **Navigation Restrictions (`on_navigation`)**: Prevents untrusted guest pages from navigating to unauthorized origins or protocols (`file://`, `tauri://`).
   * **Popup Suppression (`on_new_window`)**: Intercepts `window.open` and `<a target="_blank">` to prevent rogue window breakouts.
4. **Pipelined Pre-Refresh**: Initiates a native background reload (`Webview::reload()`) on the upcoming tile during minimization to pipeline content updates before expansion, guarded by event-correlated generation tokens against stale completions.
5. **Full-Resident Single-Window Grid**: Allocates native child webviews for all $N$ configured sites ($M = N$) so every endpoint is simultaneously visible in the grid without outer scrolling, guarded by a configurable safety ceiling (`max_resident_webviews: 12`, max 16).
6. **AdGuard DNS Adblocking**: Routes child webview requests through an embedded loopback proxy (supporting SOCKS5, HTTP CONNECT, and plain HTTP forwarding) resolving via AdGuard DNS-over-HTTPS (`https://dns.adguard-dns.com/dns-query`) with AdGuard plain UDP fallback (`94.140.14.14:53`), with user opt-in to system DNS.
7. **High Configurability & Shadowing Protection**:
   * Resolution Precedence: CLI/Env override (`--config`) > portable bundle-adjacent config (read-only) > writable OS AppData preferences > compiled defaults.
   * Source-Aware Safety: When a portable override is active, UI settings operate in read-only mode with "Export / Save As" to prevent silent shadowing; in-place saves write to AppData only when running without overrides.
---

## Documentation

* **[Architecture Specification](docs/ARCHITECTURE.md)**: Deep dive into the desktop architecture comparison (Electron `WebContentsView` vs. Tauri v2 `Webview`), security threat model, auto-tiling grid geometry, and coordinate interpolation motion mechanics.
* **[Implementation Plan](docs/IMPLEMENTATION_PLAN.md)**: Six-phase engineering roadmap from project scaffolding to tour state machine, UI controls, and performance validation.
* **[Manual Testing Guide](docs/MANUAL_TESTING.md)**: Runtime verification checklist for 5-feed layout, HUD overlay, hotkeys, settings, and focus management.
* **[Operational Logging](docs/LOGGING.md)**: Log locations, refresh event meanings, retention, privacy, and debug logging.

---

## Project Structure

```
ambient-kiosk/
├── docs/
│   ├── ARCHITECTURE.md          # Comprehensive architecture & security spec
│   ├── IMPLEMENTATION_PLAN.md   # Six-phase engineering roadmap & cross-platform guide
│   └── MANUAL_TESTING.md        # Runtime manual verification checklist
├── src-tauri/                   # Rust backend (Tauri v2, proxy, tour state machine)
├── src/                         # Frontend controller UI (Vite + TypeScript)
├── LICENSE                      # zlib-style permissive project license
└── README.md                    # Project overview & quickstart
```

---

## Prerequisites & Cross-Platform Build Guide

* **Rust & Cargo**: >= 1.90 (`rustc --version`)
* **Bun**: >= 1.0 (`bun --version`) — required runtime and package manager (invoked by Tauri build scripts and project lockfile).
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

### Windows 11 x64 Portable ZIP

Run `powershell.exe -ExecutionPolicy Bypass -File scripts\build-windows-portable.ps1`
from a clean checkout on Windows 11 x64. The output ZIP contains the app and
Microsoft's Fixed Version WebView2 runtime; see [Windows portable instructions](docs/WINDOWS_PORTABLE.md).
---

## License

This project is licensed under the terms described in the [LICENSE](LICENSE) file.

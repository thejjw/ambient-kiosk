# Implementation Plan: Ambient Kiosk

## Overview

This document outlines the step-by-step implementation roadmap for building **Ambient Kiosk** as a high-performance Tauri v2 desktop application.

---

## Technical Stack

* **Host Framework**: Tauri v2 (`tauri` ^2.0)
* **Backend**: Rust (`std::sync`, `serde`, `tauri::webview::WebviewBuilder`)
* **Frontend Controller**: TypeScript + Vite (minimal footprint, zero bloated component libraries)
* **Package Manager**: Bun (`bun` 1.4.x)
* **Target Platforms**: macOS (WKWebView), Windows (WebView2), Linux (WebKitGTK)

---

## Phase Breakdown
### Phase 0: macOS Feasibility Spike (Native Webview Realities)
* **Objective**: Validate native child `Webview` behaviors, z-ordering limits, and adblocking proxy before full application wiring.
* **Spike Steps**:
  1. **Dual-Webview Bounds & Maximization**:
     * Spawn 2 child `Webview`s inside a test window.
     * Animate webview 0 to full window dimensions.
     * Validate sibling hide fallback (`webview1.hide()`) to prevent visual collision, since Tauri v2 lacks a cross-platform `set_z_order` API.
     * Verify smooth restoration (`webview1.show()`) upon return to resting bounds.
  2. **Controller HUD & Occlusion**:
     * Prove HUD visibility using a reserved $40\,\text{px}$ top bar ($y \in [0, 40]$) versus a secondary frameless overlay window with `always_on_top(true)`.
  3. **Pipelined JIT Reload & PreparingNext Readiness**:
     * Trigger background `next_webview.reload()` on idle view, mark pending reload, and handle the first subsequent `on_page_load(Finished)` event.
     * Validate internal `tour_generation: u64` token to invalidate stale timeout tasks and state transitions.
     * Test `PreparingNext` state: verify advance on `Finished` event, evaluate cross-navigation/redirect race handling, and verify timeout (2.5s) skips expansion leaving view unmaximized in its resting slot.
  4. **Adblocking DNS Smoke Test**:
     * Test local HTTP/SOCKS5 proxy routing webview traffic through AdGuard DoH (`https://dns.adguard-dns.com/dns-query`) or plain DNS fallback (`94.140.14.14:53`) via `WebviewBuilder.proxy_url()`.
     * Verify macOS WKWebView proxy behavior and evaluate stability across macOS versions.
  5. **Interaction Observation Check**:
     * Test whether guest cross-origin click/scroll can be detected; confirm global keyboard shortcut (`Space` to pause/resume) as primary reliable control.
* **Verification**: Functional prototype demonstrating bounds animation, sibling hide/show, JIT native reload with timeout skipping, and proxy adblock evaluation.

### Phase 1: Tauri v2 Project Scaffolding
* Initialize Vite + TypeScript project with Bun in `ambient-kiosk`.
* Configure Tauri v2 core dependencies and capabilities.
* Set up window configuration in `tauri.conf.json`:
  * Main coordinator window (`width: 1920`, `height: 1080`, `resizable: true`, `decorations: true`).
  * Background color set to `#121212` to prevent native resize flashes.
* **Verification**: `cargo check` and `bun run build` succeed; minimal window renders.
### Phase 2: Configuration & Persistence Layer
* Define Rust data structures with `serde`:
  * `KioskConfig`, `WindowConfig`, `LayoutConfig`, `TimingConfig`, `TourConfig`, `NetworkDnsConfig`, `LimitsConfig`, `EndpointItem`.
* Implement configuration resolution precedence & platform isolation:
  1. **CLI / Environment Override**: `--config <path>` flag or `KIOSK_CONFIG=<path>` environment variable.
  2. **Platform-Resolved Portable Config (Read-Only Override)**:
     * On macOS: `kiosk-config.json` adjacent to `AmbientKiosk.app` (resolved by traversing up from `Contents/MacOS` past the bundle root). Never auto-write into or generate files in the bundle directory, preserving code-signing integrity.
     * On Windows/Linux: `kiosk-config.json` adjacent to the executable binary.
     * Treated strictly as a read-only override when detected.
  3. **User Application Support Directory (Writable Preferences)**:
     * macOS: `~/Library/Application Support/ambient-kiosk/config.json`
     * Windows: `%APPDATA%\ambient-kiosk\config.json`
     * Linux: `~/.config/ambient-kiosk/config.json`
     * Standard target for in-app preference saves when no portable override is active.
  4. **Compiled Defaults**: Built-in presets used if no external configuration file is present.
* Implement Source-Aware Persistence & Shadowing Protection:
  * Backend returns config metadata: `source: "cli" | "portable" | "appdata" | "defaults"`, `is_readonly: bool`.
  * If active source is portable/CLI: Settings UI is read-only with an "Export / Save As..." action; in-place saves return an explicit error to prevent silent shadowing.
  * If active source is app-data or defaults: In-place saves persist directly to user Application Support.
* Seed default endpoints with the 5 presets:
  1. Biztoc (`https://biztoc.com/`)
  2. Alltoc (`https://alltoc.com/`)
  3. Biztoc Wire (`https://biztoc.com/wire`)
  4. AP News Latest (`https://apnews.com/hub/latest-news`)
  5. Finviz News (`https://finviz.com/news`)
* Expose Tauri commands:
  * `get_config() -> KioskConfig`
  * `save_config(config: KioskConfig) -> Result<(), String>`
* **Verification**: Unit tests for resolution precedence (CLI > portable read-only > writable app-data > defaults), serialization/deserialization, rejection of writes into signed bundle directories, and assertion that `save_config` rejects shadowed saves when a higher-precedence portable source is active.

### Phase 3: Native Multi-Webview Orchestration & Adblocking Proxy
* Implement embedded DNS-resolving local proxy in Rust:
  * Routes DNS queries to AdGuard DoH/DoT/UDP.
  * Exposes local HTTP/SOCKS5 proxy on `127.0.0.1:<port>` when `adblock_dns_enabled` is active.
* Implement scroll-free dynamic $M \times N$ auto-fitting geometry calculator:
  * Dynamically computes row count $R$ and per-row column counts $c_r$ such that $\sum c_r = N$, scaling from $1 \times 1$ up to arbitrary $N$ (e.g. $[3, 2]$ for $N = 5$, $3 \times 2$ for $N = 6$, $4 \times 2$ for $N = 8$, $3 \times 3$ for $N = 9$).
  * Supports `strategy: "auto"` evaluated against configurable `target_tile_aspect_ratio` (default 1.4), with optional explicit `rows: [c0, c1, ...]` override.
  * Full-bleed default ($p = 0, g = 0$) guaranteeing 100% window client area coverage with zero outer scrollbars or clipping.
  * Configurable optional header reservation ($y \in [0, \text{header\_height}]$) when HUD is enabled.
* Implement full-resident webview lifecycle manager:
  * Allocates resident native child webviews for all $N$ configured endpoints ($M = N$) so every site renders simultaneously without scrolling.
  * Enforces `max_resident_webviews` validation (default 12) during config parsing to prevent memory/compositor exhaustion.
  * Limits in-flight reloads to at most 1 (the single upcoming target tile).
* Implement Security Policies:
  * `on_navigation`: Restrict URLs strictly to `http`/`https` protocols.
  * `on_new_window`: Intercept popup requests; cancel popups or delegate to system browser.
  * Zero-capabilities ACL: Confirm guest webview labels are excluded from all IPC command capabilities.
* **Verification**: Launch app with the 5 preset endpoints; verify all 5 render simultaneously without iframe blocking errors, with ad filtering active if Phase 0 proxy validation succeeded or unproxied system DNS if proxying was rejected.

### Phase 4: Animation & Tour Engine with JIT Pre-Refresh
* Implement Alternating Tour State Machine in Rust:
  * States: `Stopped`, `GridView`, `Maximizing(index)`, `MaximizedSingleSite(index)`, `Minimizing(index)`, `PreparingNext(index)`, `Paused`.
* Alternating Tour Cadence:
  1. **`GridView`**: Hold all sites visible in full-bleed layout for `grid_view_duration_ms` (e.g. 20s).
  2. **`Maximizing(i)`**: Interpolate bounds of focused webview from resting slot to $(0, 0, W, H)$ using cubic ease-in-out curve ($e(t) = 3t^2 - 2t^3$) over `transition_duration_ms` (500ms). Sibling-hide fallback hides inactive views.
  3. **`MaximizedSingleSite(i)`**: Hold single expanded site for `maximized_hold_duration_ms` (e.g. 30s).
  4. **`Minimizing(i)`**: Interpolate bounds back to resting slot. Invoke native `next_webview.reload()` on candidate $(i + 1) \pmod N$ if idle.
  5. **`PreparingNext(i + 1)`**: Transition upon minimization completion.
     * On `on_page_load(Finished)`: Clear pending flag, return to `GridView` for `grid_view_duration_ms`, then advance to `Maximizing(i + 1)`.
     * On timeout (2.5s) or load failure: Leave incomplete view unmaximized in resting slot, clear pending flag, return to `GridView`, and advance candidate to $(i + 2) \pmod N$.
* Expose control commands:
  * `start_tour()`, `pause_tour()`, `resume_tour()`, `next_tile()`, `prev_tile()`.
* **Verification**: Observe alternating cycle (20s full-bleed multi-site grid $\to$ 30s maximized single site $\to$ 20s multi-site grid $\to$ next maximized site), with background reload and unmaximized slot timeout behavior.

### Phase 5: Controller UI & User Interaction
* Build an unobtrusive floating HUD / control overlay:
  * Reserved non-overlapping top bar ($y \in [0, 40]$) or frameless always-on-top overlay.
  * Indicator badges showing active tile title, countdown timer, and adblock status.
  * Hotkeys: `Space` (pause/resume), `ArrowRight` (next), `ArrowLeft` (previous), `F11` (fullscreen).
  * Settings drawer to add, reorder, delete URLs, adjust durations, and toggle DNS adblocking.
* Interaction detection & pause:
  * User interaction pauses the tour timer; resumes after idle timeout (e.g. 15s).
* **Verification**: Verify manual navigation, pause/resume hotkeys, and URL configuration drawer.

### Phase 6: Hardening, Performance & Documentation
* Memory and process profiling:
  * CPU and memory profiling across hardware: `[Pending manual profiling across targets]`
  * Audio muting verification: `[Pending manual runtime audio check]`
  * Verified bounded pool ceiling (`max_resident_webviews <= 16`) prevents runaway process allocation via automated unit test.
* Cross-platform verification:
  * macOS: Automated suites passing (29 unit tests, 5 UI tests); manual runtime checklist documented in `docs/MANUAL_TESTING.md`.
  * Windows & Linux: `[Pending user environment compilation and testing]`
* Documentation:
  * Completed architecture specification, implementation plan, manual testing checklist, and cross-platform guide.
---

## Cross-Platform Implementation Guide (Windows & Linux)

This guide specifies how to tackle Windows and Linux compilation, testing, and deployment, highlighting what core components work out-of-the-box, where platform divergences exist, and what specific work is required for implementors on each target operating system.

### 1. Core Cross-Platform Capabilities (Work Out-of-the-Box)

The following core modules are implemented using standard, portable Rust and cross-platform Tauri v2 APIs that require no architectural changes for Windows or Linux:

* **Configuration & Precedence Engine (`src-tauri/src/config.rs`)**:
  * Resolves configuration hierarchy (`CLI > Portable > AppData > Defaults`) uniformly.
  * Portable config discovery (`find_portable_config_path`) automatically checks `kiosk-config.json` adjacent to `ambient-kiosk.exe` on Windows and the `ambient-kiosk` binary on Linux.
  * User AppData directory (`app_config_dir()`) resolves automatically to `%APPDATA%\com.ambientkiosk.kiosk` on Windows and `~/.config/com.ambientkiosk.kiosk` on Linux (XDG specification).
  * Startup safety validation (`max_resident_webviews <= 16`, title/URL validation, hex color parsing) is 100% portable.
* **Aspect-Ratio Geometry Calculator (`src-tauri/src/layout.rs`)**:
  * Pure mathematical candidate scoring algorithm calculating full-bleed slot bounds. Identical across all window managers and display types.
* **Alternating Tour State Machine (`src-tauri/src/tour.rs`)**:
  * State transitions, timing countdowns, generation token tracking, stalled-load safety skipping, and paused visual state calculations operate purely in memory and on Tokio asynchronous timers.
* **Embedded Loopback Proxy (`src-tauri/src/proxy.rs`)**:
  * Multiplexed HTTP CONNECT, SOCKS5 CONNECT, and plain HTTP forwarding built with standard `tokio::net` async sockets (`TcpListener`, `TcpStream`, `UdpSocket`).
  * Resolves upstream using AdGuard DoH (RFC 8484 wire format) with fallback to AdGuard plain UDP (`94.140.14.14:53`).
  * `WebviewBuilder.proxy_url("http://127.0.0.1:<port>")` is supported natively by WebView2 on Windows and WebKitGTK 4.1 on Linux.
* **Frontend Controller UI (`src/`)**:
  * Vite + TypeScript frontend, HUD controls, and safe DOM endpoint management render identically inside the coordinator webview across all platforms.

---

### 2. Platform Divergences & Implementor Guidance

Implementors deploying or testing on Windows and Linux must understand the following platform-specific behaviors:

#### A. Linux (Ubuntu / Debian / Fedora / Arch)

1. **Global Shortcuts on Wayland vs. X11**:
   * *Architecture*: The global shortcut subsystem (`global-hotkey 0.8` used by `tauri-plugin-global-shortcut`) implements native key grabbing for **X11 only** via `x11rb`.
   * *Wayland Limitation*: Under pure Wayland sessions (without XWayland active), global hotkey registration will fail or return an error because `global-hotkey 0.8` does not implement an XDG Desktop Portal (`org.freedesktop.portal.GlobalShortcuts`) backend.
   * *Implementor Action*:
     * In X11 sessions (GNOME on Xorg, XFCE, i3): Hotkeys (`Space`, arrows, `F11`, `Escape`, `KeyH`) function out-of-the-box.
     * In pure Wayland sessions: The application handles shortcut registration failure gracefully without crashing. Users on pure Wayland must rely directly on the on-screen HUD buttons (`◀`, `⏸`, `▶`, `⛶`, `⚙`).
     * Future Extension: Implement native DBus portal communication with `org.freedesktop.portal.GlobalShortcuts` to support Wayland global key grabs.

2. **Window Transparency vs. Hit-Testing (Click-Through)**:
   * *Architecture*: Window visual transparency (`transparent: true` in `tauri.conf.json`) and OS hit-testing/click-through (`set_ignore_cursor_events: true`) are separate mechanisms:
     * `transparent: true` governs visual alpha blending with the desktop.
     * `set_ignore_cursor_events: true` instructs the OS compositor to ignore pointer clicks and pass them through to underlying guest webviews.
   * *Linux Requirement*: Visual transparency on Linux requires an active EWMH compositing window manager (e.g., Mutter on GNOME, KWin on KDE, Picom/Compton on tiling WMs).
   * *Implementor Action*:
     * If deployed on a bare X11 window manager without a compositor (e.g. bare i3 or Openbox), transparent window regions will render as solid black rectangles. Ensure a compositor like Picom is running, or set `transparent: false` with background `#0d0d0d` in `tauri.conf.json`.

3. **System Dependencies & Packaging**:
   * *Required Development Libraries*:
     * Debian/Ubuntu: `libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev`
     * Fedora: `webkit2gtk4.1-devel openssl-devel curl wget file libappindicator-gtk3-devel librsvg2-devel`
     * Arch Linux: `webkit2gtk-4.1 base-devel curl wget file openssl libappindicator-gtk3 librsvg`
   * *Packaging Formats*: Use `bun run tauri build` to generate native `.deb`, `.AppImage`, or `.rpm` packages.

---

#### B. Windows 10 / 11

1. **Child Webview Hosting & WebView2 Runtime**:
   * *Architecture*: Tauri v2 uses Microsoft Edge WebView2 (Chromium engine) via Win32 child `HWND`s.
   * *System Requirement*: WebView2 runtime is pre-installed on Windows 10 and 11. For Windows Server or stripped environments, install the Evergreen WebView2 Bootstrapper.
   * *Per-Webview Proxying*: `WebviewBuilder::proxy_url("http://127.0.0.1:<port>")` is supported natively by WebView2 without requiring special compilation flags.

2. **Window Background & Transparency**:
   * *Behavior*: Win32 ignores alpha channels on standard window surfaces, but WebView2 supports transparent background composition natively.
   * *Implementor Action*: The HUD overlay window (`hud-overlay`) floats above the main window with `alwaysOnTop: true`. On Windows, ensure `SetWindowPos` or Tauri window sizing synchronizes smoothly during multi-DPI monitor changes.

3. **High-DPI Coordinate Systems (Per-Monitor V2)**:
   * *Architecture*: Windows uses Per-Monitor V2 DPI scaling. `main_win.cursor_position()` and `outer_position()` return desktop physical coordinates.
   * *Implementor Action*: Verify that moving the window across monitors with mixed scaling (e.g., a 4K display at 150% and a 1080p display at 100%) correctly recalculates logical window bounds via `main_win.scale_factor()`.

4. **Console Window Subsystem**:
   * *Setup*: `src-tauri/src/main.rs` includes `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`, ensuring no background command prompt window is spawned in release builds.

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

### Phase 1: Tauri v2 Project Scaffolding
* Initialize Vite + TypeScript project with Bun in `ambient-kiosk`.
* Configure Tauri v2 core dependencies and capabilities.
* Set up window configuration in `tauri.conf.json`:
  * Main coordinator window (`width: 1920`, `height: 1080`, `resizable: true`, `decorations: true`).
  * Background color set to `#121212` to prevent native resize flashes.
* **Verification**: `cargo check` and `bun run build` succeed; minimal window renders.

### Phase 2: Configuration & Persistence Layer
* Define Rust data structures with `serde`:
  * `KioskConfig`, `KioskSettings`, `EndpointItem`.
* Implement local JSON configuration loading and saving in `$APPDATA/ambient-kiosk/config.json`.
* Seed default endpoints for first-time launch:
  * Hacker News (`https://news.ycombinator.com`)
  * GitHub Trending (`https://github.com/trending`)
  * BBC News (`https://www.bbc.com/news`)
  * Weather Radar (`https://radar.weather.gov`)
* Expose Tauri commands:
  * `get_config() -> KioskConfig`
  * `save_config(config: KioskConfig) -> Result<(), String>`
* **Verification**: Unit tests for config serialization/deserialization and fallback to defaults.

### Phase 3: Native Multi-Webview Orchestration
* Implement grid geometry calculator in Rust:
  * Given $N$ endpoints and main window inner size $(W, H)$, calculate bounding boxes $(x, y, w, h)$ for slots $0 \dots N-1$.
* Implement webview lifecycle manager:
  * `spawn_guest_webviews(app_handle, endpoints)`:
    * Construct child `WebviewBuilder` per endpoint.
    * Assign unique webview labels (`guest-0`, `guest-1`, etc.).
    * Set initial resting bounds.
* Implement Security Policies:
  * `on_navigation`: Restrict URLs strictly to `http`/`https` protocols.
  * `on_new_window`: Intercept popup requests; cancel popups or delegate to system browser.
  * Zero-capabilities ACL: Confirm guest webview labels are excluded from all IPC command capabilities.
* **Verification**: Launch app with 4 test endpoints; verify all 4 render simultaneously without iframe blocking errors.

### Phase 4: Animation & Tour Engine
* Implement Tour State Machine in Rust:
  * States: `Stopped`, `GridRest`, `Maximizing(index)`, `Maximized(index)`, `Minimizing(index)`, `Paused`.
* Implement Coordinate Interpolator:
  * Cubic ease-in-out curve: $e(t) = 3t^2 - 2t^3$.
  * Timer loop running intermediate bounds updates on the active webview from $(x_{\text{rest}}, y_{\text{rest}}, w_{\text{rest}}, h_{\text{rest}})$ to $(0, 0, W, H)$ over configured transition duration (e.g. 500ms).
* Implement Tour Timer:
  * After `Maximizing` finishes, transition to `Maximized`.
  * Start hold timer (e.g. 30 seconds).
  * On expiration, transition to `Minimizing`.
  * Advance active index: $i_{\text{next}} = (i + 1) \pmod N$.
* Expose control commands:
  * `start_tour()`, `pause_tour()`, `resume_tour()`, `next_tile()`, `prev_tile()`.
* **Verification**: Observe smooth expansion from grid slot to full screen, hold for configured seconds, and contraction back to slot.

### Phase 5: Controller UI & User Interaction
* Build an unobtrusive floating HUD / control overlay:
  * Hover-activated top bar or hotkey (`Space` for pause/resume, `ArrowRight` for next, `ArrowLeft` for previous, `F11` for fullscreen).
  * Indicator badges showing current active tile number and countdown timer.
  * Settings drawer to add, reorder, delete URLs and adjust hold/transition durations.
* Interaction detection:
  * If user hovers or interacts with the maximized view, temporarily pause the countdown timer to allow reading/scrolling.
  * Resume countdown after idle timeout (e.g. 15s).
* **Verification**: Verify manual navigation, pause/resume hotkeys, and URL configuration drawer.

### Phase 6: Hardening, Performance & Documentation
* Memory and process profiling:
  * Monitor CPU and memory with 6+ live webviews.
  * Verify audio is muted on guest webviews by default.
* Cross-platform smoke testing:
  * macOS (WKWebView): test full-screen transitions and multi-monitor setups.
  * Windows (WebView2): verify layout sizing without border jitter.
* Write developer and user documentation:
  * Quickstart guide, keyboard shortcuts, configuration manual.

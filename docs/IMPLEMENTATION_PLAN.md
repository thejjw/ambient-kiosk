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
  * `KioskConfig`, `KioskSettings`, `EndpointItem`.
* Implement local JSON configuration loading and saving in `$APPDATA/ambient-kiosk/config.json`.
* Seed default endpoints with the 5 presets:
  1. Biztoc (`https://biztoc.com/`)
  2. Alltoc (`https://alltoc.com/`)
  3. Biztoc Wire (`https://biztoc.com/wire`)
  4. AP News Latest (`https://apnews.com/hub/latest-news`)
  5. Finviz News (`https://finviz.com/news`)
* Include DNS adblocking settings (`adblock_dns_enabled`, `dns_provider`, `doh_url`, `dot_url`, `plain_dns_ip`).
* Expose Tauri commands:
  * `get_config() -> KioskConfig`
  * `save_config(config: KioskConfig) -> Result<(), String>`
* **Verification**: Unit tests for config serialization/deserialization and fallback to defaults.

### Phase 3: Native Multi-Webview Orchestration & Adblocking Proxy
* Implement embedded DNS-resolving local proxy in Rust:
  * Routes DNS queries to AdGuard DoH/DoT/UDP.
  * Exposes local HTTP/SOCKS5 proxy on `127.0.0.1:<port>` when `adblock_dns_enabled` is active.
* Implement asymmetric grid geometry calculator:
  * 3/2 box layout for $N = 5$ (Row 0: 3 columns, Row 1: 2 columns).
  * Respects reserved $40\,\text{px}$ header zone for controller HUD.
* Implement bounded webview pool manager:
  * For baseline $N = 5$, maintain all 5 visible webviews resident simultaneously.
  * Apply virtualization and slot re-navigation only when configured URLs exceed visible slots ($M = 5$).
  * Limit in-flight reloads to at most 1 (the single upcoming target tile).
* Implement Security Policies:
  * `on_navigation`: Restrict URLs strictly to `http`/`https` protocols.
  * `on_new_window`: Intercept popup requests; cancel popups or delegate to system browser.
  * Zero-capabilities ACL: Confirm guest webview labels are excluded from all IPC command capabilities.
* **Verification**: Launch app with the 5 preset endpoints; verify all 5 render simultaneously without iframe blocking errors, with ad filtering active if Phase 0 proxy validation succeeded.

### Phase 4: Animation & Tour Engine with JIT Pre-Refresh
* Implement Tour State Machine in Rust:
  * States: `Stopped`, `GridRest`, `Maximizing(index)`, `Maximized(index)`, `Minimizing(index)`, `PreparingNext(index)`, `Paused`.
* Implement Coordinate Interpolator:
  * Cubic ease-in-out curve: $e(t) = 3t^2 - 2t^3$.
  * Timer loop updating bounds of focused webview from resting slot to $(0, 0, W, H)$.
  * Sibling-hide fallback: hide sibling webviews during maximization, restore on minimizing completion.
* Implement JIT Pre-Refresh Trigger & PreparingNext State:
  * Upon entering `Minimizing(i)`, verify target webview is idle, mark pending reload, invoke `next_webview.reload()`, and increment internal `tour_generation: u64`.
  * Upon completing minimization, transition to `PreparingNext(i + 1)`.
  * Await the first `on_page_load(Finished)` on the pending view: on arrival, clear pending flag, advance to `GridRest` and `Maximizing(i + 1)`.
  * On timeout (2.5s safety limit, checked against current `tour_generation`) or load failure: leave the incomplete view unmaximized in its resting slot, clear pending flag, and advance to prepare candidate $i + 2$.
  * Hold timer (e.g. 30 seconds).
  * Advance active index: $i_{\text{next}} = (i + 1) \pmod N$.
* Expose control commands:
  * `start_tour()`, `pause_tour()`, `resume_tour()`, `next_tile()`, `prev_tile()`.
* **Verification**: Observe smooth expansion from grid slot to full screen, hold for configured seconds, background refresh of upcoming tile, and contraction back to slot.

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
  * Monitor CPU and memory with the 5 preset feeds running.
  * Confirm audio is muted by default across all guest webviews.
  * Verify bounded pool prevents runaway process allocation.
* Cross-platform smoke testing:
  * macOS (WKWebView): test full-screen transitions and multi-monitor setups.
  * Windows (WebView2): verify layout sizing without border jitter.
* Documentation:
  * Architecture, developer guide, keyboard shortcuts, configuration manual.

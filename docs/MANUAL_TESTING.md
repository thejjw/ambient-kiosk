# Manual Testing Guide: Ambient Kiosk

This checklist guides manual review and verification of **Ambient Kiosk** runtime behaviors on desktop environments (macOS, Windows, Linux).

---

## 1. Launching the Application

### Development Mode (with Live Reload)
```bash
cd ambient-kiosk
bun run tauri dev
```

### Production Debug Bundle (macOS)
```bash
open "ambient-kiosk/src-tauri/target/debug/bundle/macos/Ambient Kiosk.app"
```

---

## 2. Verification Checklist

### A. 5-Preset Full-Bleed Grid Layout
* [ ] **Concurrent Rendering**: All 5 preset web feeds render simultaneously in an auto-fitting $3 \times 2$ grid:
  1. Biztoc (`https://biztoc.com/`)
  2. Alltoc (`https://alltoc.com/`)
  3. Biztoc Wire (`https://biztoc.com/wire`)
  4. AP News Latest (`https://apnews.com/hub/latest-news`)
  5. Finviz News (`https://finviz.com/news`)
* [ ] **Bypass Framing Headers**: Pages load without `X-Frame-Options` or CSP `frame-ancestors` blocking errors.
* [ ] **100% Canvas Coverage**: Full-bleed geometry ($p = 0, g = 0$) fills the entire window client area without outer scrollbars or clipped edges.

---

### B. HUD Overlay & Cursor Interaction
* [ ] **Initial Boot Reveal**: On startup, the HUD overlay displays at the top ($y \in [0, 48]$) showing the status badge (`GRID VIEW`), site title, countdown progress bar, and control buttons, then auto-hides after 2.5s.
* [ ] **Top-Edge Cursor Activation**: Move the mouse cursor to within 20 logical px of the top window edge ($y \le 20$). The HUD overlay slides down and becomes interactive.
* [ ] **Native Click-Through**: When the mouse cursor moves outside the top 52px band, the HUD overlay becomes click-through (`ignore_cursor_events: true`), allowing mouse clicks to pass directly into underlying guest web content.
* [ ] **`H` Hotkey Toggle**: Press `H` to toggle HUD visibility and interactivity on/off.

---

### C. Alternating Tour Cycle
* [ ] **Overview Cadence**: Grid holds all 5 sites in overview for configured duration (default 20s).
* [ ] **Maximization Motion**: Upcoming tile smoothly expands to full screen over 500ms using a cubic ease-in-out motion curve ($e(t) = 3t^2 - 2t^3$). Inactive sibling tiles hide (`hide()`) during maximization.
* [ ] **Reading Hold**: Maximized single-site view holds for configured duration (default 30s) with active site title in HUD.
* [ ] **Minimization & Background Reload**: Tile minimizes back to its grid slot, sibling tiles restore (`show()`), and upcoming candidate tile reloads in the background.

---

### D. Keyboard Hotkeys & Controls
* [ ] **`Space` (Play / Pause)**: Press `Space` to pause the tour. The play/pause button icon toggles to `▶`, and the countdown progress bar freezes. Press `Space` again to resume (button icon returns to `⏸`).
* [ ] **`ArrowRight` (Next Tile)**: Press `ArrowRight` to advance directly to the next tile in the tour.
* [ ] **`ArrowLeft` (Previous Tile)**: Press `ArrowLeft` to return to the previous tile.
* [ ] **`F11` (or `[⛶]` Button)**: Toggle fullscreen mode. Confirm all 5 tiles reflow dynamically to fit the new monitor dimensions and the HUD overlay width updates.
* [ ] **`Escape`**: Closes the settings drawer if open, or minimizes an expanded tile back to its resting slot.

---

### E. Settings Drawer & Persistence
* [ ] **Opening Drawer**: Click the settings gear icon (`[⚙]`) on the HUD overlay. The settings drawer slides in from the right edge.
* [ ] **720p / Small Display Scroll**: On smaller viewports (e.g. 1280x720), verify that the drawer body scrolls smoothly (`overflow-y: auto`) so all configuration groups, endpoint rows, and drawer action buttons remain reachable.
* [ ] **Source Badge**:
  * If launched with `--config` or portable `kiosk-config.json`: Displays `Source: CLI / Portable [READ ONLY]`. All inputs, reorder buttons, and "Save Changes" are disabled. "Export Configuration..." is active.
  * If launched with standard defaults/AppData: Inputs are editable and "Save Changes" is enabled.
* [ ] **Endpoint List Management**:
  * Edit website titles and URLs.
  * Reorder feeds using `↑` and `↓` buttons.
  * Delete feeds using `✕` (disabled when only 1 feed remains).
  * Add feeds using `+ Add Endpoint` (disabled when reaching `max_resident_webviews`).
* [ ] **Saving Preferences**: Click "Save Changes" to atomically persist configuration to `~/Library/Application Support/com.ambientkiosk.kiosk/config.json`.
* [ ] **Exporting Configuration**: Click "Export Configuration..." to save a portable JSON configuration file to a custom destination.

---

### F. Focus Synchronization & OS Safety
* [ ] **Focus Loss (Blur)**: Switch focus away to another application (Finder, browser, editor). Confirm that the floating HUD overlay immediately hides and all global shortcuts (`Space`, `ArrowRight`, `ArrowLeft`, `F11`, `Escape`, `H`) are completely unregistered so they never intercept typing in other applications.
* [ ] **Focus Return**: Switch back to Ambient Kiosk. Confirm that the HUD overlay restores and shortcuts re-register cleanly.

---

## 3. macOS Security & Keychain Note

* **Debug Build Keychain Prompt**: When running unsigned debug builds on macOS, third-party guest web feeds (such as AP News with Turnstile) may cause WebKit to request access to an existing WebCrypto master key in the login keychain.
* **Action**: Clicking **"Deny"** dismisses the dialog and allows the kiosk session to continue without granting keychain access.

# Ambient Kiosk for Windows 11 x64

Extract the entire ZIP to a local folder, then launch `ambient-kiosk.exe`. Keep the
`runtime` folder beside the executable. No installer or WebView2 download is
required to launch the app. The web feeds and the default AdGuard DNS proxy
require network access.

The app starts with five built-in feeds. Open Settings with H or the HUD gear.
Saving editable settings writes to `%APPDATA%\com.ambientkiosk.kiosk\config.json`.
Restart the app after saving so all webviews, proxy, and tour settings use the
new values. Browser data is stored under `%LOCALAPPDATA%\com.ambientkiosk.kiosk`.

To use a managed configuration, copy `kiosk-config.example.json` to
`kiosk-config.json` beside `ambient-kiosk.exe`, then edit it and restart. This
adjacent configuration takes precedence over AppData and makes in-app Settings
read-only; use Export Configuration to save a new file. You can also launch
`ambient-kiosk.exe --config "C:\path with spaces\config.json"` for an explicit
override.

To update, close the app and extract the new ZIP into a new local folder. Your
AppData preferences remain available. Copy an existing adjacent
`kiosk-config.json` into the new folder if you use one. To remove the app,
delete its extracted folder. To remove preferences and browser data as well,
delete the two application folders under `%APPDATA%` and `%LOCALAPPDATA%` after
closing the app.

The bundled WebView2 runtime is a fixed version and receives updates only
when a new Ambient Kiosk ZIP is provided. Launch from a local drive: Microsoft
does not support running this runtime from a network share or UNC path.

## Building the portable ZIP

On Windows 11 x64, install Bun, Rust with the MSVC target, and the Visual C++
toolchain with Windows SDK. Run `bun install --frozen-lockfile`, then run
`powershell.exe -ExecutionPolicy Bypass -File scripts\build-windows-portable.ps1`
from the repository. The script verifies a pinned Microsoft WebView2 CAB,
builds the release executable, and writes a ZIP, SHA-256 file, and build
manifest to the workspace `artifacts` folder. It downloads the CAB only if
the pinned file is absent. Build from a clean committed checkout.

The ZIP is unsigned. Windows may show a SmartScreen prompt when opening a
downloaded unsigned executable.

## Validation status

On the Windows 11 build machine, `bun test` passed 5 UI tests, Rust passed
30 unit tests, and `bun run tauri build --no-bundle --ci` produced the release
executable. The portable ZIP hash and required files were checked. A local
launch loaded live web content using the bundled WebView2 runtime; all of the
app-owned browser processes exited when that test run stopped.

The checklist in `MANUAL_TESTING.md` remains open for interactive HUD, mixed
DPI, offline launch without Evergreen WebView2, and extended tour testing.

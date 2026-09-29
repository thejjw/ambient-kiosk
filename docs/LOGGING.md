# Operational logging

Ambient Kiosk writes structured JSON Lines to its per-user application log directory. The same location is used by installed and portable builds.

## Find the log

Open Settings and look under **Operational Diagnostics** for the active log file path and current logging health. Tauri resolves the directory for each platform and honors `appDirectoriesOverride`:

- Windows: `%LOCALAPPDATA%\com.ambientkiosk.kiosk\logs`
- macOS: `~/Library/Logs/com.ambientkiosk.kiosk`
- Linux: `~/.local/share/com.ambientkiosk.kiosk/logs`

The active file is `ambient-kiosk.jsonl`. The app rotates it at 5 MiB and retains up to five older files. One log record may make a file slightly larger than the threshold. Together, the active file and archives use about 30 MiB at most.

## Event format and refresh meanings

Each JSON line has a schema version, UTC epoch timestamp in milliseconds, session ID, increasing event sequence, severity, component, event name, and a JSON-encoded `details` field. The Settings summary shows uptime, tour state, proxy/DNS totals, logger health, and each feed's latest refresh status for this run.

Refresh records distinguish a requested reload, an observed page-load start, a finished callback, dispatch failure, timeout, and tracking cancellation. Load-start observations and ignored callbacks are debug detail; terminal outcomes remain at the default info level. `reload_finished_observed` means the webview emitted the matching Started/Finished callback sequence. It does not prove an HTTP response succeeded or that the publisher changed its content. Tauri does not provide a navigation request ID in this callback, so a delayed overlapping navigation can be associated with the current attempt. The preparing timeout starts after the minimize transition; the logged elapsed time starts when the reload is requested.

The health summary is recorded every 60 seconds. Settings refreshes its in-memory summary every two seconds while open. It stops polling when closed. A warning appears if the log directory cannot be opened, a file write fails, or the nonblocking queue drops records; refresh status remains available in memory for the current run.

## Troubleshooting and privacy

Logging defaults to `info`. For additional application detail, set `KIOSK_LOG_LEVEL=debug` before launching:

```powershell
$env:KIOSK_LOG_LEVEL = "debug"
.\ambient-kiosk.exe
```

Debug mode adds event details but does not collect animation frames, cursor polling, or guest-page console output. The file logger accepts only Ambient Kiosk diagnostic events; dependency and network-stack records are filtered out even in debug mode. Persistent records exclude URLs, hostnames, page text, headers, credentials, raw configuration, and exception messages. Refresh records identify feeds by configured endpoint ID and tile index.

The logger uses a bounded nonblocking queue. Its Settings health counters report dropped records and file write errors. Orderly application exit flushes queued records; forced termination can leave the last queued records unwritten.

## Not included

This logging feature observes the current tour-triggered reload behavior. `reload_interval_minutes` and `refresh_before_maximize` still do not schedule reloads. Per-feed scheduling, pause interaction, and verification that website content actually changed remain future work.

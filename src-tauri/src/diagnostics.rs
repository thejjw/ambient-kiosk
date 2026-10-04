use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager, State, WebviewWindow};
use tracing::Level;
use tracing_appender::non_blocking::{ErrorCounter, NonBlockingBuilder, WorkerGuard};
use tracing_subscriber::filter::{LevelFilter, Targets};
use tracing_subscriber::prelude::*;

const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;
const ARCHIVE_COUNT: usize = 5;

/// Current-session diagnostics displayed in the trusted settings webview.
#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticsSnapshot {
    pub session_id: String,
    pub uptime_seconds: u64,
    pub log_path: String,
    pub storage_available: bool,
    pub write_errors: u64,
    pub dropped_records: usize,
    pub tour_state: String,
    pub active_endpoint_id: Option<String>,
    pub proxy_enabled: bool,
    pub proxy_connections: u64,
    pub dns_resolved: u64,
    pub dns_blocked: u64,
    pub dns_failed: u64,
    pub feeds: Vec<FeedRefreshStatus>,
}

/// Most recent refresh information for one configured endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct FeedRefreshStatus {
    pub endpoint_id: String,
    pub tile_index: usize,
    pub last_attempt_at_ms: Option<u128>,
    pub last_completed_at_ms: Option<u128>,
    pub last_outcome: Option<String>,
    pub elapsed_ms: Option<u64>,
    pub pending: bool,
}

#[derive(Debug, Clone)]
struct RefreshAttempt {
    generation: u64,
    tile_index: usize,
    endpoint_id: String,
    trigger: String,
    requested_at_ms: u128,
    started: Option<Instant>,
    requested: Instant,
}

struct WriterHealth {
    storage_available: AtomicBool,
    write_errors: AtomicU64,
}

/// Process-wide logging and in-memory operational summary.
pub struct DiagnosticsState {
    session_id: String,
    started: Instant,
    log_path: String,
    event_sequence: AtomicU64,
    writer_health: Arc<WriterHealth>,
    dropped_records: ErrorCounter,
    worker_guard: Mutex<Option<WorkerGuard>>,
    shutdown_started: AtomicBool,
    feeds: Mutex<Vec<FeedRefreshStatus>>,
    pending_refresh: Mutex<Option<RefreshAttempt>>,
    tour_state: Mutex<String>,
    active_endpoint_id: Mutex<Option<String>>,
    proxy_enabled: AtomicBool,
    refresh_totals: [AtomicU64; 4],
    proxy_totals: [AtomicU64; 4],
}

/// Returns current diagnostics only to the trusted settings webview.
#[tauri::command]
pub fn get_diagnostics(
    webview_window: WebviewWindow,
    diagnostics: State<'_, Arc<DiagnosticsState>>,
) -> Result<DiagnosticsSnapshot, String> {
    if webview_window.label() != "hud-overlay" {
        return Err("Only the HUD can read diagnostics".into());
    }
    Ok(diagnostics.snapshot())
}

/// Records a fixed error category from the trusted HUD without storing exception text.
#[tauri::command]
pub fn report_frontend_error(
    webview_window: WebviewWindow,
    diagnostics: State<'_, Arc<DiagnosticsState>>,
    category: String,
) -> Result<(), String> {
    if webview_window.label() != "hud-overlay" {
        return Err("Only the HUD can report frontend errors".into());
    }
    if !is_allowed_frontend_error_category(&category) {
        return Err("Unsupported frontend error category".into());
    }
    diagnostics.event(
        Level::WARN,
        "frontend",
        "frontend_error",
        serde_json::json!({ "category": category }),
    );
    Ok(())
}

fn is_allowed_frontend_error_category(category: &str) -> bool {
    matches!(
        category,
        "window_error"
            | "promise_rejection"
            | "settings_load_failed"
            | "settings_save_failed"
            | "settings_export_failed"
    )
}

impl DiagnosticsState {
    /// Installs the structured logger before configuration and webview startup.
    pub fn initialize(app: &AppHandle) -> Arc<Self> {
        let session_id = format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let requested_level = std::env::var("KIOSK_LOG_LEVEL")
            .ok()
            .filter(|level| level.eq_ignore_ascii_case("debug"))
            .map(|_| LevelFilter::DEBUG)
            .unwrap_or(LevelFilter::INFO);
        let app_log_dir = app.path().app_log_dir().ok();
        let log_path = app_log_dir
            .as_ref()
            .map(|dir| dir.join("ambient-kiosk.jsonl"))
            .unwrap_or_else(|| PathBuf::from("application log directory unavailable"));

        let writer_health = Arc::new(WriterHealth {
            storage_available: std::sync::atomic::AtomicBool::new(app_log_dir.is_some()),
            write_errors: AtomicU64::new(0),
        });
        let writer: Box<dyn Write + Send> = app_log_dir
            .as_deref()
            .and_then(|dir| create_rotating_writer(dir, writer_health.clone()).ok())
            .map(|writer| Box::new(writer) as Box<dyn Write + Send>)
            .unwrap_or_else(|| {
                // Retain the in-memory health and refresh summary if persistent storage is unavailable.
                writer_health
                    .storage_available
                    .store(false, Ordering::Relaxed);
                writer_health.write_errors.fetch_add(1, Ordering::Relaxed);
                Box::new(CountingSink(writer_health.clone()))
            });
        let (non_blocking, guard) = NonBlockingBuilder::default()
            .buffered_lines_limit(4096)
            .lossy(true)
            .finish(writer);
        let dropped_records = non_blocking.error_counter();
        let logger_installed = build_subscriber(non_blocking, requested_level)
            .try_init()
            .is_ok();

        let state = Arc::new(Self {
            session_id,
            started: Instant::now(),
            log_path: log_path.display().to_string(),
            event_sequence: AtomicU64::new(0),
            writer_health,
            dropped_records,
            worker_guard: Mutex::new(Some(guard)),
            shutdown_started: AtomicBool::new(false),
            feeds: Mutex::new(Vec::new()),
            pending_refresh: Mutex::new(None),
            tour_state: Mutex::new("Stopped".into()),
            active_endpoint_id: Mutex::new(None),
            proxy_enabled: AtomicBool::new(false),
            refresh_totals: std::array::from_fn(|_| AtomicU64::new(0)),
            proxy_totals: std::array::from_fn(|_| AtomicU64::new(0)),
        });
        state.event(
            Level::INFO,
            "application",
            "startup",
            serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "platform": std::env::consts::OS,
                "log_level": if requested_level == LevelFilter::DEBUG { "debug" } else { "info" },
                "logger_installed": logger_installed,
                "storage_available": state.writer_health.storage_available.load(Ordering::Relaxed),
            }),
        );
        if !logger_installed {
            state
                .writer_health
                .write_errors
                .fetch_add(1, Ordering::Relaxed);
        }
        let panic_diagnostics = state.clone();
        let previous_panic_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let (line, column) = info
                .location()
                .map(|location| (location.line(), location.column()))
                .unwrap_or((0, 0));
            panic_diagnostics.event(
                Level::ERROR,
                "runtime",
                "panic",
                serde_json::json!({ "line": line, "column": column }),
            );
            previous_panic_hook(info);
        }));
        state
    }

    /// Records a structured operational event without including raw URLs or site content.
    pub fn event(&self, level: Level, component: &str, name: &str, details: Value) {
        let event_sequence = self.event_sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u64::MAX as u128) as u64;
        let details = serde_json::to_string(&details).unwrap_or_else(|_| "{}".into());
        match level {
            Level::ERROR => {
                tracing::error!(schema_version = 1, timestamp_ms, session_id = %self.session_id, event_sequence, component, event = name, details = %details)
            }
            Level::WARN => {
                tracing::warn!(schema_version = 1, timestamp_ms, session_id = %self.session_id, event_sequence, component, event = name, details = %details)
            }
            Level::DEBUG => {
                tracing::debug!(schema_version = 1, timestamp_ms, session_id = %self.session_id, event_sequence, component, event = name, details = %details)
            }
            _ => {
                tracing::info!(schema_version = 1, timestamp_ms, session_id = %self.session_id, event_sequence, component, event = name, details = %details)
            }
        }
    }

    /// Initializes the per-session feed summary after active configuration is resolved.
    pub fn set_feeds(&self, endpoint_ids: &[String], proxy_enabled: bool) {
        let mut feeds = self.feeds.lock();
        *feeds = endpoint_ids
            .iter()
            .enumerate()
            .map(|(tile_index, endpoint_id)| FeedRefreshStatus {
                endpoint_id: endpoint_id.clone(),
                tile_index,
                last_attempt_at_ms: None,
                last_completed_at_ms: None,
                last_outcome: None,
                elapsed_ms: None,
                pending: false,
            })
            .collect();
        self.proxy_enabled.store(proxy_enabled, Ordering::Relaxed);
    }

    /// Records the current tour state for the next Settings diagnostics response.
    pub fn set_tour_state(&self, state: String, active_index: usize) {
        let previous = std::mem::replace(&mut *self.tour_state.lock(), state.clone());
        let active_endpoint = self
            .feeds
            .lock()
            .iter()
            .find(|feed| feed.tile_index == active_index)
            .map(|feed| feed.endpoint_id.clone());
        *self.active_endpoint_id.lock() = active_endpoint.clone();
        if previous != state {
            self.event(
                Level::INFO,
                "tour",
                "tour_state_changed",
                serde_json::json!({ "previous_state": previous, "state": state, "active_endpoint_id": active_endpoint }),
            );
        }
    }

    /// Starts refresh tracking and records the trigger before invoking the webview.
    pub fn begin_refresh(&self, tile_index: usize, generation: u64, trigger: &str) {
        if let Some(previous) = self.pending_refresh.lock().take() {
            self.finish_refresh_attempt(previous, "tracking_cancelled", "superseded");
        }
        let mut feeds = self.feeds.lock();
        let Some(feed) = feeds.iter_mut().find(|f| f.tile_index == tile_index) else {
            return;
        };
        let attempt = RefreshAttempt {
            generation,
            tile_index,
            endpoint_id: feed.endpoint_id.clone(),
            trigger: trigger.into(),
            requested_at_ms: unix_millis(),
            started: None,
            requested: Instant::now(),
        };
        feed.last_attempt_at_ms = Some(attempt.requested_at_ms);
        feed.last_outcome = Some("pending".into());
        feed.elapsed_ms = None;
        feed.pending = true;
        drop(feeds);
        self.event(
            Level::INFO,
            "refresh",
            "refresh_requested",
            serde_json::json!({ "endpoint_id": attempt.endpoint_id, "tile_index": tile_index, "attempt_id": generation, "trigger": trigger }),
        );
        *self.pending_refresh.lock() = Some(attempt);
    }

    /// Associates a page-load start observation with the current refresh attempt when possible.
    pub fn observe_refresh_started(&self, tile_index: usize, generation: Option<u64>) {
        let mut pending = self.pending_refresh.lock();
        let Some(attempt) = pending.as_mut().filter(|attempt| {
            attempt.tile_index == tile_index && Some(attempt.generation) == generation
        }) else {
            return;
        };
        if attempt.started.is_some() {
            return;
        }
        attempt.started = Some(Instant::now());
        self.event(
            Level::DEBUG,
            "refresh",
            "refresh_load_started_observed",
            serde_json::json!({ "endpoint_id": attempt.endpoint_id, "tile_index": tile_index, "attempt_id": attempt.generation, "trigger": attempt.trigger }),
        );
    }

    /// Finishes the matching attempt once and updates the current-session feed summary.
    pub fn finish_refresh(&self, generation: u64, outcome: &str, reason: Option<&str>) {
        let mut pending = self.pending_refresh.lock();
        if pending
            .as_ref()
            .is_none_or(|attempt| attempt.generation != generation)
        {
            return;
        }
        if let Some(attempt) = pending.take() {
            self.finish_refresh_attempt(attempt, outcome, reason.unwrap_or(""));
        }
    }

    fn finish_refresh_attempt(&self, attempt: RefreshAttempt, outcome: &str, reason: &str) {
        let elapsed_ms = attempt
            .requested
            .elapsed()
            .as_millis()
            .min(u64::MAX as u128) as u64;
        if let Some(feed) = self
            .feeds
            .lock()
            .iter_mut()
            .find(|f| f.tile_index == attempt.tile_index)
        {
            if outcome == "finish_observed" {
                feed.last_completed_at_ms = Some(unix_millis());
            }
            feed.last_outcome = Some(outcome.into());
            feed.elapsed_ms = Some(elapsed_ms);
            feed.pending = false;
        }
        let total_index = match outcome {
            "finish_observed" => 0,
            "dispatch_failed" => 1,
            "wait_timed_out" => 2,
            _ => 3,
        };
        self.refresh_totals[total_index].fetch_add(1, Ordering::Relaxed);
        let event_name = match outcome {
            "finish_observed" => "reload_finished_observed",
            "dispatch_failed" => "reload_dispatch_failed",
            "wait_timed_out" => "reload_wait_timed_out",
            _ => "reload_tracking_cancelled",
        };
        self.event(
            if outcome == "dispatch_failed" || outcome == "wait_timed_out" { Level::WARN } else { Level::INFO },
            "refresh",
            event_name,
            serde_json::json!({ "endpoint_id": attempt.endpoint_id, "tile_index": attempt.tile_index, "attempt_id": attempt.generation, "trigger": attempt.trigger, "outcome": outcome, "elapsed_ms": elapsed_ms, "reason": reason }),
        );
    }

    /// Adds an aggregate proxy or DNS outcome without recording the requested host.
    pub fn record_proxy_outcome(&self, outcome: &str) {
        let index = match outcome {
            "connection" => 0,
            "resolved" => 1,
            "blocked" => 2,
            _ => 3,
        };
        self.proxy_totals[index].fetch_add(1, Ordering::Relaxed);
    }

    /// Captures the current process health and aggregate network outcomes in one log event.
    pub fn record_health_summary(&self) {
        let snapshot = self.snapshot();
        self.event(
            Level::INFO,
            "health",
            "health_summary",
            serde_json::json!({
                "uptime_seconds": snapshot.uptime_seconds,
                "tour_state": snapshot.tour_state,
                "resident_webviews": snapshot.feeds.len(),
                "refresh_finished": self.refresh_totals[0].load(Ordering::Relaxed),
                "refresh_dispatch_failed": self.refresh_totals[1].load(Ordering::Relaxed),
                "refresh_timed_out": self.refresh_totals[2].load(Ordering::Relaxed),
                "refresh_cancelled": self.refresh_totals[3].load(Ordering::Relaxed),
                "proxy_connections": snapshot.proxy_connections,
                "dns_resolved": snapshot.dns_resolved,
                "dns_blocked": snapshot.dns_blocked,
                "dns_failed": snapshot.dns_failed,
                "log_write_errors": snapshot.write_errors,
                "log_dropped_records": snapshot.dropped_records,
            }),
        );
    }

    /// Builds the in-memory diagnostics summary for the trusted Settings UI.
    pub fn snapshot(&self) -> DiagnosticsSnapshot {
        DiagnosticsSnapshot {
            session_id: self.session_id.clone(),
            uptime_seconds: self.started.elapsed().as_secs(),
            log_path: self.log_path.clone(),
            storage_available: self.writer_health.storage_available.load(Ordering::Relaxed),
            write_errors: self.writer_health.write_errors.load(Ordering::Relaxed),
            dropped_records: self.dropped_records.dropped_lines(),
            tour_state: self.tour_state.lock().clone(),
            active_endpoint_id: self.active_endpoint_id.lock().clone(),
            proxy_enabled: self.proxy_enabled.load(Ordering::Relaxed),
            proxy_connections: self.proxy_totals[0].load(Ordering::Relaxed),
            dns_resolved: self.proxy_totals[1].load(Ordering::Relaxed),
            dns_blocked: self.proxy_totals[2].load(Ordering::Relaxed),
            dns_failed: self.proxy_totals[3].load(Ordering::Relaxed),
            feeds: self.feeds.lock().clone(),
        }
    }

    /// Records that the process is shutting down and keeps the nonblocking worker alive to flush.
    pub fn shutdown(&self) {
        if self.shutdown_started.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Some(attempt) = self.pending_refresh.lock().take() {
            self.finish_refresh_attempt(attempt, "tracking_cancelled", "shutdown");
        }
        self.event(
            Level::INFO,
            "application",
            "shutdown",
            serde_json::json!({ "uptime_seconds": self.started.elapsed().as_secs() }),
        );
        self.worker_guard.lock().take();
    }
}

fn build_subscriber<W>(writer: W, max_level: LevelFilter) -> impl tracing::Subscriber + Send + Sync
where
    W: for<'writer> tracing_subscriber::fmt::writer::MakeWriter<'writer> + Send + Sync + 'static,
{
    tracing_subscriber::registry().with(
        tracing_subscriber::fmt::layer()
            .json()
            .with_ansi(false)
            .with_writer(writer)
            .with_filter(Targets::new().with_target("ambient_kiosk_lib::diagnostics", max_level)),
    )
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn create_rotating_writer(
    directory: &Path,
    health: Arc<WriterHealth>,
) -> io::Result<RotatingWriter> {
    fs::create_dir_all(directory)?;
    let path = directory.join("ambient-kiosk.jsonl");
    let file = OpenOptions::new().create(true).append(true).open(&path)?;
    let current_bytes = file.metadata()?.len();
    Ok(RotatingWriter {
        path,
        file: Some(file),
        current_bytes,
        health,
    })
}

struct RotatingWriter {
    path: PathBuf,
    file: Option<File>,
    current_bytes: u64,
    health: Arc<WriterHealth>,
}

impl RotatingWriter {
    fn rotate(&mut self) -> io::Result<()> {
        if let Some(mut file) = self.file.take() {
            if let Err(error) = file.flush() {
                self.file = Some(file);
                return Err(error);
            }
        }
        for index in (1..=ARCHIVE_COUNT).rev() {
            let source = if index == 1 {
                self.path.clone()
            } else {
                archive_path(&self.path, index - 1)
            };
            let destination = archive_path(&self.path, index);
            if destination.exists() {
                fs::remove_file(&destination)?;
            }
            if source.exists() {
                fs::rename(&source, &destination)?;
            }
        }
        self.file = Some(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)?,
        );
        self.current_bytes = 0;
        Ok(())
    }
}

fn archive_path(path: &Path, index: usize) -> PathBuf {
    path.with_file_name(format!("ambient-kiosk.jsonl.{index}"))
}

impl Write for RotatingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.current_bytes > 0
            && self.current_bytes.saturating_add(bytes.len() as u64) > MAX_LOG_BYTES
        {
            if let Err(error) = self.rotate() {
                self.health
                    .storage_available
                    .store(false, Ordering::Relaxed);
                self.health.write_errors.fetch_add(1, Ordering::Relaxed);
                self.file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.path)
                    .ok();
                return Err(error);
            }
        }
        let Some(file) = self.file.as_mut() else {
            self.health
                .storage_available
                .store(false, Ordering::Relaxed);
            self.health.write_errors.fetch_add(1, Ordering::Relaxed);
            return Err(io::Error::other("log file is unavailable"));
        };
        let result = file.write_all(bytes);
        match result {
            Ok(()) => {
                self.current_bytes += bytes.len() as u64;
                Ok(bytes.len())
            }
            Err(error) => {
                self.health
                    .storage_available
                    .store(false, Ordering::Relaxed);
                self.health.write_errors.fetch_add(1, Ordering::Relaxed);
                Err(error)
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        let Some(file) = self.file.as_mut() else {
            self.health
                .storage_available
                .store(false, Ordering::Relaxed);
            self.health.write_errors.fetch_add(1, Ordering::Relaxed);
            return Err(io::Error::other("log file is unavailable"));
        };
        match file.flush() {
            Ok(()) => Ok(()),
            Err(error) => {
                self.health
                    .storage_available
                    .store(false, Ordering::Relaxed);
                self.health.write_errors.fetch_add(1, Ordering::Relaxed);
                Err(error)
            }
        }
    }
}

struct CountingSink(Arc<WriterHealth>);

impl Write for CountingSink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let _ = &self.0;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state() -> Arc<DiagnosticsState> {
        let health = Arc::new(WriterHealth {
            storage_available: AtomicBool::new(true),
            write_errors: AtomicU64::new(0),
        });
        let (writer, guard) = NonBlockingBuilder::default().finish(io::sink());
        let dropped_records = writer.error_counter();
        drop(writer);
        Arc::new(DiagnosticsState {
            session_id: "test-session".into(),
            started: Instant::now(),
            log_path: "test.jsonl".into(),
            event_sequence: AtomicU64::new(0),
            writer_health: health,
            dropped_records,
            worker_guard: Mutex::new(Some(guard)),
            shutdown_started: AtomicBool::new(false),
            feeds: Mutex::new(Vec::new()),
            pending_refresh: Mutex::new(None),
            tour_state: Mutex::new("Stopped".into()),
            active_endpoint_id: Mutex::new(None),
            proxy_enabled: AtomicBool::new(false),
            refresh_totals: std::array::from_fn(|_| AtomicU64::new(0)),
            proxy_totals: std::array::from_fn(|_| AtomicU64::new(0)),
        })
    }

    #[derive(Clone)]
    struct SharedWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::writer::MakeWriter<'a> for SharedWriter {
        type Writer = SharedWriter;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    #[test]
    fn events_are_single_line_json_with_refresh_fields_and_no_url() {
        let state = test_state();
        let output = SharedWriter(Arc::new(Mutex::new(Vec::new())));
        let subscriber = build_subscriber(output.clone(), LevelFilter::DEBUG);

        tracing::subscriber::with_default(subscriber, || {
            tracing::debug!(target: "reqwest::connect", hostname = "private.example", ip = "192.0.2.10");
            state.event(
                Level::INFO,
                "refresh",
                "refresh_requested",
                serde_json::json!({ "endpoint_id": "feed-1", "attempt_id": 42, "trigger": "tour_advance" }),
            );
        });

        let bytes = output.0.lock().clone();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.ends_with('\n'));
        assert_eq!(text.lines().count(), 1);
        assert!(!text.contains("url"));
        assert!(!text.contains("private.example"));
        assert!(!text.contains("192.0.2.10"));
        let record: Value = serde_json::from_str(text.trim()).unwrap();
        let fields = record.get("fields").unwrap_or(&record);
        assert_eq!(fields["schema_version"], 1);
        assert_eq!(fields["session_id"], "test-session");
        assert_eq!(fields["event_sequence"], 1);
        assert_eq!(fields["component"], "refresh");
        assert_eq!(fields["event"], "refresh_requested");
        assert!(fields["timestamp_ms"].as_u64().is_some());
        let details: Value = serde_json::from_str(fields["details"].as_str().unwrap()).unwrap();
        assert_eq!(details["endpoint_id"], "feed-1");
        assert_eq!(details["attempt_id"], 42);
    }

    #[test]
    fn worker_guard_flushes_queued_records_on_drop() {
        let output = SharedWriter(Arc::new(Mutex::new(Vec::new())));
        let (mut writer, guard) = NonBlockingBuilder::default().finish(output.clone());
        writer.write_all(b"last-record\n").unwrap();
        drop(writer);
        drop(guard);
        assert_eq!(&*output.0.lock(), b"last-record\n");
    }

    #[test]
    fn timeout_ignores_late_finish_and_duplicate_terminal_outcomes() {
        let state = test_state();
        state.set_feeds(&["feed-1".into()], false);
        state.begin_refresh(0, 42, "tour_advance");
        state.finish_refresh(42, "wait_timed_out", None);
        state.finish_refresh(42, "finish_observed", None);

        let snapshot = state.snapshot();
        assert_eq!(
            snapshot.feeds[0].last_outcome.as_deref(),
            Some("wait_timed_out")
        );
        assert!(snapshot.feeds[0].last_completed_at_ms.is_none());
        assert!(!snapshot.feeds[0].pending);
        assert_eq!(state.refresh_totals[0].load(Ordering::Relaxed), 0);
        assert_eq!(state.refresh_totals[2].load(Ordering::Relaxed), 1);
    }

    #[test]
    fn superseding_refresh_cancels_the_previous_attempt() {
        let state = test_state();
        state.set_feeds(&["feed-1".into(), "feed-2".into()], false);
        state.begin_refresh(0, 41, "tour_advance");
        state.begin_refresh(1, 42, "manual_minimize");

        let snapshot = state.snapshot();
        assert_eq!(
            snapshot.feeds[0].last_outcome.as_deref(),
            Some("tracking_cancelled")
        );
        assert!(!snapshot.feeds[0].pending);
        assert!(snapshot.feeds[1].pending);
        assert_eq!(state.refresh_totals[3].load(Ordering::Relaxed), 1);
    }

    #[test]
    fn shutdown_retires_pending_refresh_before_late_callbacks() {
        let state = test_state();
        state.set_feeds(&["feed-1".into()], false);
        state.begin_refresh(0, 42, "tour_advance");
        state.shutdown();
        let shutdown_sequence = state.event_sequence.load(Ordering::Relaxed);
        state.shutdown();
        assert_eq!(
            state.event_sequence.load(Ordering::Relaxed),
            shutdown_sequence
        );
        state.finish_refresh(42, "finish_observed", None);

        let snapshot = state.snapshot();
        assert_eq!(
            snapshot.feeds[0].last_outcome.as_deref(),
            Some("tracking_cancelled")
        );
        assert!(!snapshot.feeds[0].pending);
        assert!(snapshot.feeds[0].last_completed_at_ms.is_none());
        assert_eq!(state.refresh_totals[3].load(Ordering::Relaxed), 1);
        assert_eq!(state.refresh_totals[0].load(Ordering::Relaxed), 0);
    }

    #[test]
    fn competing_terminal_outcomes_record_exactly_one_result() {
        let state = test_state();
        state.set_feeds(&["feed-1".into()], false);
        state.begin_refresh(0, 42, "tour_advance");
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let finish_state = state.clone();
        let finish_barrier = barrier.clone();
        let finish = std::thread::spawn(move || {
            finish_barrier.wait();
            finish_state.finish_refresh(42, "finish_observed", None);
        });
        let timeout_state = state.clone();
        let timeout_barrier = barrier.clone();
        let timeout = std::thread::spawn(move || {
            timeout_barrier.wait();
            timeout_state.finish_refresh(42, "wait_timed_out", None);
        });
        barrier.wait();
        finish.join().unwrap();
        timeout.join().unwrap();

        let terminal_count = state.refresh_totals[0].load(Ordering::Relaxed)
            + state.refresh_totals[2].load(Ordering::Relaxed);
        assert_eq!(terminal_count, 1);
        assert!(!state.snapshot().feeds[0].pending);
    }

    #[test]
    fn rotating_writer_keeps_only_five_archives() {
        let directory = std::env::temp_dir().join(format!("ambient-kiosk-logs-{}", unix_millis()));
        let health = Arc::new(WriterHealth {
            storage_available: AtomicBool::new(true),
            write_errors: AtomicU64::new(0),
        });
        let mut writer = create_rotating_writer(&directory, health.clone()).unwrap();
        for _ in 0..7 {
            writer.current_bytes = MAX_LOG_BYTES;
            writer.write_all(b"record\n").unwrap();
        }
        assert!(archive_path(&writer.path, ARCHIVE_COUNT).is_file());
        assert!(!archive_path(&writer.path, ARCHIVE_COUNT + 1).exists());
        assert_eq!(health.write_errors.load(Ordering::Relaxed), 0);
        drop(writer);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn writer_failures_are_visible_in_health_state() {
        let directory =
            std::env::temp_dir().join(format!("ambient-kiosk-log-failure-{}", unix_millis()));
        let health = Arc::new(WriterHealth {
            storage_available: AtomicBool::new(true),
            write_errors: AtomicU64::new(0),
        });
        let mut writer = create_rotating_writer(&directory, health.clone()).unwrap();
        writer.file = None;

        assert!(writer.write_all(b"record\n").is_err());
        assert!(!health.storage_available.load(Ordering::Relaxed));
        assert_eq!(health.write_errors.load(Ordering::Relaxed), 1);

        drop(writer);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn nonblocking_queue_reports_dropped_records() {
        struct BlockingWriter {
            started: std::sync::mpsc::Sender<()>,
            release: std::sync::mpsc::Receiver<()>,
        }

        impl Write for BlockingWriter {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                let _ = self.started.send(());
                let _ = self.release.recv();
                Ok(bytes.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let writer = BlockingWriter {
            started: started_tx,
            release: release_rx,
        };
        let (mut nonblocking, guard) = NonBlockingBuilder::default()
            .buffered_lines_limit(1)
            .lossy(true)
            .finish(writer);
        nonblocking.write_all(b"first").unwrap();
        started_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        nonblocking.write_all(b"queued").unwrap();
        nonblocking.write_all(b"dropped").unwrap();
        assert_eq!(nonblocking.error_counter().dropped_lines(), 1);

        release_tx.send(()).unwrap();
        drop(guard);
    }

    #[test]
    fn diagnostics_summary_starts_with_empty_feed_history() {
        let snapshot = DiagnosticsSnapshot {
            session_id: "test".into(),
            uptime_seconds: 0,
            log_path: String::new(),
            storage_available: true,
            write_errors: 0,
            dropped_records: 0,
            tour_state: "Stopped".into(),
            active_endpoint_id: None,
            proxy_enabled: false,
            proxy_connections: 0,
            dns_resolved: 0,
            dns_blocked: 0,
            dns_failed: 0,
            feeds: vec![FeedRefreshStatus {
                endpoint_id: "one".into(),
                tile_index: 0,
                last_attempt_at_ms: None,
                last_completed_at_ms: None,
                last_outcome: None,
                elapsed_ms: None,
                pending: false,
            }],
        };
        assert_eq!(snapshot.feeds[0].last_outcome, None);
        assert!(!snapshot.feeds[0].pending);
    }

    #[test]
    fn frontend_error_categories_are_allowlisted() {
        assert!(is_allowed_frontend_error_category("window_error"));
        assert!(is_allowed_frontend_error_category("promise_rejection"));
        assert!(!is_allowed_frontend_error_category(
            "https://private.example/path?token=x"
        ));
        assert!(!is_allowed_frontend_error_category(
            "stack trace or page content"
        ));
    }
}

use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};
use tracing::Level;
use tracing_appender::non_blocking::{ErrorCounter, NonBlockingBuilder, WorkerGuard};
use tracing_subscriber::filter::LevelFilter;

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
    pub feeds: Vec<FeedRefreshStatus>,
}

/// Most recent refresh information for one configured endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct FeedRefreshStatus {
    pub endpoint_id: String,
    pub tile_index: usize,
    pub last_attempt_at_ms: Option<u128>,
    pub last_finished_at_ms: Option<u128>,
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
    feeds: Mutex<Vec<FeedRefreshStatus>>,
    pending_refresh: Mutex<Option<RefreshAttempt>>,
    tour_state: Mutex<String>,
    active_endpoint_id: Mutex<Option<String>>,
    proxy_enabled: AtomicBool,
    refresh_totals: [AtomicU64; 4],
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
                writer_health.storage_available.store(false, Ordering::Relaxed);
                writer_health.write_errors.fetch_add(1, Ordering::Relaxed);
                Box::new(CountingSink(writer_health.clone()))
            });
        let (non_blocking, guard) = NonBlockingBuilder::default()
            .buffered_lines_limit(4096)
            .lossy(true)
            .finish(writer);
        let dropped_records = non_blocking.error_counter();
        let logger_installed = tracing_subscriber::fmt()
            .json()
            .with_ansi(false)
            .with_max_level(requested_level)
            .with_writer(non_blocking)
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
            feeds: Mutex::new(Vec::new()),
            pending_refresh: Mutex::new(None),
            tour_state: Mutex::new("Stopped".into()),
            active_endpoint_id: Mutex::new(None),
            proxy_enabled: AtomicBool::new(false),
            refresh_totals: std::array::from_fn(|_| AtomicU64::new(0)),
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
            state.writer_health.write_errors.fetch_add(1, Ordering::Relaxed);
        }
        state
    }

    /// Records a structured operational event without including raw URLs or site content.
    pub fn event(&self, level: Level, component: &str, name: &str, details: Value) {
        let event_sequence = self.event_sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let details = serde_json::to_string(&details).unwrap_or_else(|_| "{}".into());
        match level {
            Level::ERROR => tracing::error!(schema_version = 1, timestamp_ms, session_id = %self.session_id, event_sequence, component, event = name, details = %details),
            Level::WARN => tracing::warn!(schema_version = 1, timestamp_ms, session_id = %self.session_id, event_sequence, component, event = name, details = %details),
            Level::DEBUG => tracing::debug!(schema_version = 1, timestamp_ms, session_id = %self.session_id, event_sequence, component, event = name, details = %details),
            _ => tracing::info!(schema_version = 1, timestamp_ms, session_id = %self.session_id, event_sequence, component, event = name, details = %details),
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
                last_finished_at_ms: None,
                last_outcome: None,
                elapsed_ms: None,
                pending: false,
            })
            .collect();
        self.proxy_enabled.store(proxy_enabled, Ordering::Relaxed);
    }

    /// Records the current tour state for the next Settings diagnostics response.
    pub fn set_tour_state(&self, state: String, active_index: usize) {
        *self.tour_state.lock() = state;
        *self.active_endpoint_id.lock() = self
            .feeds
            .lock()
            .iter()
            .find(|feed| feed.tile_index == active_index)
            .map(|feed| feed.endpoint_id.clone());
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
        attempt.started = Some(Instant::now());
        self.event(
            Level::INFO,
            "refresh",
            "refresh_load_started_observed",
            serde_json::json!({ "endpoint_id": attempt.endpoint_id, "tile_index": tile_index, "attempt_id": attempt.generation, "trigger": attempt.trigger }),
        );
    }

    /// Finishes the matching attempt once and updates the current-session feed summary.
    pub fn finish_refresh(&self, generation: u64, outcome: &str, reason: Option<&str>) {
        let mut pending = self.pending_refresh.lock();
        if pending.as_ref().is_none_or(|attempt| attempt.generation != generation) {
            return;
        }
        if let Some(attempt) = pending.take() {
            self.finish_refresh_attempt(attempt, outcome, reason.unwrap_or(""));
        }
    }

    fn finish_refresh_attempt(&self, attempt: RefreshAttempt, outcome: &str, reason: &str) {
        let elapsed_ms = attempt.requested.elapsed().as_millis().min(u64::MAX as u128) as u64;
        if let Some(feed) = self.feeds.lock().iter_mut().find(|f| f.tile_index == attempt.tile_index) {
            feed.last_finished_at_ms = Some(unix_millis());
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
        self.event(
            if outcome == "dispatch_failed" || outcome == "wait_timed_out" { Level::WARN } else { Level::INFO },
            "refresh",
            outcome,
            serde_json::json!({ "endpoint_id": attempt.endpoint_id, "tile_index": attempt.tile_index, "attempt_id": attempt.generation, "trigger": attempt.trigger, "elapsed_ms": elapsed_ms, "reason": reason }),
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
            feeds: self.feeds.lock().clone(),
        }
    }

    /// Records that the process is shutting down and keeps the nonblocking worker alive to flush.
    pub fn shutdown(&self) {
        self.event(Level::INFO, "application", "shutdown", serde_json::json!({ "uptime_seconds": self.started.elapsed().as_secs() }));
        self.worker_guard.lock().take();
    }
}

fn unix_millis() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis()
}

fn create_rotating_writer(directory: &Path, health: Arc<WriterHealth>) -> io::Result<RotatingWriter> {
    fs::create_dir_all(directory)?;
    let path = directory.join("ambient-kiosk.jsonl");
    let file = OpenOptions::new().create(true).append(true).open(&path)?;
    let current_bytes = file.metadata()?.len();
    Ok(RotatingWriter { path, file: Some(file), current_bytes, health })
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
            let source = if index == 1 { self.path.clone() } else { archive_path(&self.path, index - 1) };
            let destination = archive_path(&self.path, index);
            if destination.exists() {
                fs::remove_file(&destination)?;
            }
            if source.exists() {
                fs::rename(&source, &destination)?;
            }
        }
        self.file = Some(OpenOptions::new().create(true).append(true).open(&self.path)?);
        self.current_bytes = 0;
        Ok(())
    }
}

fn archive_path(path: &Path, index: usize) -> PathBuf {
    path.with_file_name(format!("ambient-kiosk.jsonl.{index}"))
}

impl Write for RotatingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.current_bytes > 0 && self.current_bytes.saturating_add(bytes.len() as u64) > MAX_LOG_BYTES {
            if let Err(error) = self.rotate() {
                self.health.storage_available.store(false, Ordering::Relaxed);
                self.health.write_errors.fetch_add(1, Ordering::Relaxed);
                self.file = OpenOptions::new().create(true).append(true).open(&self.path).ok();
                return Err(error);
            }
        }
        let result = self.file.as_mut().ok_or_else(|| io::Error::other("log file is unavailable"))?.write(bytes);
        match result {
            Ok(written) => {
                self.current_bytes += written as u64;
                Ok(written)
            }
            Err(error) => {
                self.health.storage_available.store(false, Ordering::Relaxed);
                self.health.write_errors.fetch_add(1, Ordering::Relaxed);
                Err(error)
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.file.as_mut().ok_or_else(|| io::Error::other("log file is unavailable"))?.flush() {
            Ok(()) => Ok(()),
            Err(error) => {
                self.health.storage_available.store(false, Ordering::Relaxed);
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

    #[test]
    fn rotating_writer_keeps_only_five_archives() {
        let directory = std::env::temp_dir().join(format!("ambient-kiosk-logs-{}", unix_millis()));
        let health = Arc::new(WriterHealth { storage_available: AtomicBool::new(true), write_errors: AtomicU64::new(0) });
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
    fn diagnostics_summary_starts_with_empty_feed_history() {
        let snapshot = DiagnosticsSnapshot {
            session_id: "test".into(), uptime_seconds: 0, log_path: String::new(),
            storage_available: true, write_errors: 0, dropped_records: 0,
            tour_state: "Stopped".into(), active_endpoint_id: None, proxy_enabled: false,
            feeds: vec![FeedRefreshStatus { endpoint_id: "one".into(), tile_index: 0,
                last_attempt_at_ms: None, last_finished_at_ms: None, last_outcome: None,
                elapsed_ms: None, pending: false }],
        };
        assert_eq!(snapshot.feeds[0].last_outcome, None);
        assert!(!snapshot.feeds[0].pending);
    }
}

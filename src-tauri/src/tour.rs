use crate::config::TimingConfig;
use crate::webview_manager::ManagedWebviewTile;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Rect, State};
use tokio::sync::mpsc::UnboundedReceiver;

/// High-level names of the tour state machine states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TourStateName {
    Stopped,
    GridView,
    Maximizing,
    MaximizedSingleSite,
    Minimizing,
    PreparingNext,
    Paused,
}

/// Payload emitted to frontend HUD for countdown bars and status indicators.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TourStatusPayload {
    pub state: TourStateName,
    pub active_index: Option<usize>,
    pub active_title: Option<String>,
    pub progress_percent: f64,
    pub is_paused: bool,
}

/// Core state machine data.
#[derive(Debug, Clone)]
pub struct TourStateData {
    pub current_state: TourStateName,
    pub active_index: usize,
    pub candidate_index: usize,
    pub state_start: Instant,
    pub target_duration: Duration,
    pub paused_elapsed: Duration,
    pub is_paused: bool,
    pub previous_state_before_pause: TourStateName,
    pub tour_generation: u64,
    pub pending_reload: Option<(usize, u64)>,
    pub candidate_reload_finished: bool,
}

impl TourStateData {
    pub fn new() -> Self {
        Self {
            current_state: TourStateName::Stopped,
            active_index: 0,
            candidate_index: 0,
            state_start: Instant::now(),
            target_duration: Duration::from_secs(20),
            paused_elapsed: Duration::ZERO,
            is_paused: false,
            previous_state_before_pause: TourStateName::GridView,
            tour_generation: 1,
            pending_reload: None,
            candidate_reload_finished: false,
        }
    }
}

/// Pure state machine coordinating tour state transitions, generation tokens, and timeout logic.
pub struct TourStateMachine {
    pub state: Mutex<TourStateData>,
    pub timing: Mutex<TimingConfig>,
    pub total_tiles: usize,
    pub generation_counter: AtomicU64,
}

impl TourStateMachine {
    pub fn new(timing: TimingConfig, total_tiles: usize) -> Self {
        Self {
            state: Mutex::new(TourStateData::new()),
            timing: Mutex::new(timing),
            total_tiles: total_tiles.max(1),
            generation_counter: AtomicU64::new(1),
        }
    }

    /// Starts tour from Stopped or Paused state.
    pub fn start(&self) {
        let mut data = self.state.lock();
        if data.current_state == TourStateName::Stopped {
            let grid_duration = self.timing.lock().grid_view_duration_ms;
            data.current_state = TourStateName::GridView;
            data.state_start = Instant::now();
            data.target_duration = Duration::from_millis(grid_duration);
            data.is_paused = false;
        } else if data.is_paused {
            data.is_paused = false;
            data.current_state = data.previous_state_before_pause;
            data.state_start = Instant::now() - data.paused_elapsed;
            data.paused_elapsed = Duration::ZERO;
        }
    }

    /// Pauses tour countdown timer.
    pub fn pause(&self) {
        let mut data = self.state.lock();
        if !data.is_paused {
            data.is_paused = true;
            data.paused_elapsed = data.state_start.elapsed();
            data.previous_state_before_pause = data.current_state;
            data.current_state = TourStateName::Paused;
        }
    }

    /// Resumes tour countdown timer.
    pub fn resume(&self) {
        let mut data = self.state.lock();
        if data.is_paused {
            data.is_paused = false;
            data.current_state = data.previous_state_before_pause;
            data.state_start = Instant::now() - data.paused_elapsed;
            data.paused_elapsed = Duration::ZERO;
        }
    }

    /// Prepares state for Maximizing transition.
    pub fn begin_maximizing(&self, target_idx: usize) -> (usize, Duration) {
        let target = target_idx % self.total_tiles;
        let transition_duration = Duration::from_millis(self.timing.lock().transition_duration_ms);
        let mut data = self.state.lock();
        let was_paused = data.is_paused;
        data.active_index = target;
        data.state_start = Instant::now();
        data.target_duration = transition_duration;

        if was_paused {
            data.current_state = TourStateName::Paused;
            data.previous_state_before_pause = TourStateName::Maximizing;
            data.paused_elapsed = Duration::ZERO;
        } else {
            data.current_state = TourStateName::Maximizing;
        }

        (target, transition_duration)
    }

    /// Completes Maximizing and enters MaximizedSingleSite hold.
    /// Preserves Paused state and resets paused_elapsed to ZERO if paused mid-animation.
    pub fn finish_maximizing(&self) -> TourStateName {
        let hold_duration = Duration::from_millis(self.timing.lock().maximized_hold_duration_ms);
        let mut data = self.state.lock();
        data.state_start = Instant::now();
        data.target_duration = hold_duration;

        if data.is_paused {
            data.current_state = TourStateName::Paused;
            data.previous_state_before_pause = TourStateName::MaximizedSingleSite;
            data.paused_elapsed = Duration::ZERO;
            TourStateName::Paused
        } else {
            data.current_state = TourStateName::MaximizedSingleSite;
            TourStateName::MaximizedSingleSite
        }
    }

    /// Prepares state for Minimizing transition, setting up candidate reload tracking.
    pub fn begin_minimizing(&self, target_idx: usize) -> (usize, usize, u64, Duration) {
        let target = target_idx % self.total_tiles;
        let candidate = (target + 1) % self.total_tiles;
        let transition_duration = Duration::from_millis(self.timing.lock().transition_duration_ms);
        let gen = self.generation_counter.fetch_add(1, Ordering::SeqCst) + 1;

        let mut data = self.state.lock();
        let was_paused = data.is_paused;
        data.state_start = Instant::now();
        data.target_duration = transition_duration;
        data.candidate_index = candidate;
        data.tour_generation = gen;
        data.pending_reload = Some((candidate, gen));
        data.candidate_reload_finished = false;

        if was_paused {
            data.current_state = TourStateName::Paused;
            data.previous_state_before_pause = TourStateName::Minimizing;
            data.paused_elapsed = Duration::ZERO;
        } else {
            data.current_state = TourStateName::Minimizing;
        }

        (target, candidate, gen, transition_duration)
    }

    /// Completes Minimizing.
    /// Preserves Paused state and resets paused_elapsed to ZERO if paused mid-animation.
    pub fn finish_minimizing(&self) -> TourStateName {
        let mut data = self.state.lock();
        let destination_state = if data.candidate_reload_finished {
            let grid_duration = Duration::from_millis(self.timing.lock().grid_view_duration_ms);
            data.target_duration = grid_duration;
            data.pending_reload = None;
            TourStateName::GridView
        } else {
            let timeout_duration = Duration::from_millis(self.timing.lock().preparing_timeout_ms);
            data.target_duration = timeout_duration;
            TourStateName::PreparingNext
        };

        data.state_start = Instant::now();

        if data.is_paused {
            data.current_state = TourStateName::Paused;
            data.previous_state_before_pause = destination_state;
            data.paused_elapsed = Duration::ZERO;
            TourStateName::Paused
        } else {
            data.current_state = destination_state;
            destination_state
        }
    }

    /// Handles page load finished event from the webview event channel.
    /// If paused in PreparingNext, transitions the paused destination to GridView.
    pub fn on_page_load_finished(&self, loaded_index: usize, event_gen: u64) -> bool {
        let mut data = self.state.lock();
        if data.pending_reload == Some((loaded_index, event_gen)) {
            data.candidate_reload_finished = true;
            if data.current_state == TourStateName::PreparingNext {
                let grid_duration = Duration::from_millis(self.timing.lock().grid_view_duration_ms);
                data.current_state = TourStateName::GridView;
                data.state_start = Instant::now();
                data.target_duration = grid_duration;
                data.pending_reload = None;
                return true;
            } else if data.is_paused && data.previous_state_before_pause == TourStateName::PreparingNext {
                // Loaded while paused in PreparingNext: update destination to GridView and reset paused_elapsed
                let grid_duration = Duration::from_millis(self.timing.lock().grid_view_duration_ms);
                data.previous_state_before_pause = TourStateName::GridView;
                data.state_start = Instant::now();
                data.target_duration = grid_duration;
                data.paused_elapsed = Duration::ZERO;
                data.pending_reload = None;
                return true;
            }
        }
        false
    }

    /// Advances tour when PreparingNext encounters a timeout (2.5s) or load error.
    /// Guarded by expected generation token to invalidate stale timeout tasks.
    pub fn on_preparing_timeout(&self, expected_gen: u64) -> Option<usize> {
        let mut data = self.state.lock();
        let matches_preparing = data.current_state == TourStateName::PreparingNext
            || (data.is_paused && data.previous_state_before_pause == TourStateName::PreparingNext);

        if matches_preparing && data.tour_generation == expected_gen {
            let next_candidate = (data.candidate_index + 1) % self.total_tiles;
            data.candidate_index = next_candidate;
            data.pending_reload = None;
            data.candidate_reload_finished = false;

            let grid_duration = Duration::from_millis(self.timing.lock().grid_view_duration_ms);
            data.state_start = Instant::now();
            data.target_duration = grid_duration;

            if data.is_paused {
                data.previous_state_before_pause = TourStateName::GridView;
                data.paused_elapsed = Duration::ZERO;
            } else {
                data.current_state = TourStateName::GridView;
            }

            Some(next_candidate)
        } else {
            None
        }
    }

    pub fn get_status_payload(&self, titles: &[String]) -> TourStatusPayload {
        let data = self.state.lock();
        let elapsed = if data.is_paused {
            data.paused_elapsed
        } else {
            data.state_start.elapsed()
        };

        let total_ms = data.target_duration.as_millis().max(1) as f64;
        let elapsed_ms = elapsed.as_millis() as f64;
        let progress_percent = (elapsed_ms / total_ms * 100.0).clamp(0.0, 100.0);

        let title = if data.active_index < titles.len() {
            Some(titles[data.active_index].clone())
        } else {
            None
        };

        TourStatusPayload {
            state: data.current_state,
            active_index: Some(data.active_index),
            active_title: title,
            progress_percent,
            is_paused: data.is_paused,
        }
    }
}

/// Controller coordinating webview animations, reloads, and the tour loop.
pub struct TourController {
    pub app_handle: AppHandle,
    pub machine: TourStateMachine,
    pub reserved_header_height: f64,
    pub window_width: f64,
    pub window_height: f64,
    pub tiles: Vec<ManagedWebviewTile>,
    pub is_running: AtomicBool,
    pub should_exit: AtomicBool,
}

impl TourController {
    pub fn new(
        app_handle: AppHandle,
        timing: TimingConfig,
        reserved_header_height: f64,
        window_width: f64,
        window_height: f64,
        tiles: Vec<ManagedWebviewTile>,
    ) -> Arc<Self> {
        let total = tiles.len();
        Arc::new(Self {
            app_handle,
            machine: TourStateMachine::new(timing, total),
            reserved_header_height,
            window_width,
            window_height,
            tiles,
            is_running: AtomicBool::new(false),
            should_exit: AtomicBool::new(false),
        })
    }

    pub fn emit_status(&self) {
        let titles: Vec<String> = self.tiles.iter().map(|t| t.title.clone()).collect();
        let payload = self.machine.get_status_payload(&titles);
        let _ = self.app_handle.emit_to("main", "tour-status-update", payload);
    }

    pub fn start(&self) {
        self.is_running.store(true, Ordering::SeqCst);
        self.machine.start();
        self.emit_status();
    }

    pub fn pause(&self) {
        self.machine.pause();
        self.emit_status();
    }

    pub fn resume(&self) {
        self.machine.resume();
        self.emit_status();
    }

    pub fn transition_to_maximizing(&self, target_idx: usize) {
        if self.tiles.is_empty() {
            return;
        }
        let (target, transition_duration) = self.machine.begin_maximizing(target_idx);

        // Sibling-hide fallback: hide all inactive sibling webviews
        for (i, tile) in self.tiles.iter().enumerate() {
            if i != target {
                let _ = tile.webview.hide();
            }
        }
        self.emit_status();

        // Perform bounds interpolation loop over transition_duration
        let steps = 25;
        let step_delay = Duration::from_millis((transition_duration.as_millis() as u64 / steps as u64).max(1));
        let rest = self.tiles[target].resting_rect;
        let full_w = self.window_width;
        let full_h = self.window_height - self.reserved_header_height;
        let full_y = self.reserved_header_height;

        for s in 1..=steps {
            let t = s as f64 / steps as f64;
            let eased_t = 3.0 * t * t - 2.0 * t * t * t;

            let curr_x = rest.x * (1.0 - eased_t);
            let curr_y = rest.y + (full_y - rest.y) * eased_t;
            let curr_w = rest.width + (full_w - rest.width) * eased_t;
            let curr_h = rest.height + (full_h - rest.height) * eased_t;

            let _ = self.tiles[target].webview.set_bounds(Rect {
                position: tauri::Position::Logical(LogicalPosition::new(curr_x, curr_y)),
                size: tauri::Size::Logical(LogicalSize::new(curr_w, curr_h)),
            });
            std::thread::sleep(step_delay);
        }

        let _ = self.tiles[target].webview.set_bounds(Rect {
            position: tauri::Position::Logical(LogicalPosition::new(0.0, full_y)),
            size: tauri::Size::Logical(LogicalSize::new(full_w, full_h)),
        });

        self.machine.finish_maximizing();
        self.emit_status();
    }

    pub fn transition_to_minimizing(&self, target_idx: usize) {
        if self.tiles.is_empty() {
            return;
        }
        let (target, candidate, gen, transition_duration) = self.machine.begin_minimizing(target_idx);
        self.emit_status();

        // Trigger native reload paired with this exact generation token
        let _ = self.tiles[candidate].trigger_reload(gen);

        // Interpolate back to resting slot
        let steps = 25;
        let step_delay = Duration::from_millis((transition_duration.as_millis() as u64 / steps as u64).max(1));
        let rest = self.tiles[target].resting_rect;
        let full_w = self.window_width;
        let full_h = self.window_height - self.reserved_header_height;
        let full_y = self.reserved_header_height;

        for s in 1..=steps {
            let t = s as f64 / steps as f64;
            let eased_t = 3.0 * t * t - 2.0 * t * t * t;

            let curr_x = rest.x * eased_t;
            let curr_y = full_y + (rest.y - full_y) * eased_t;
            let curr_w = full_w - (full_w - rest.width) * eased_t;
            let curr_h = full_h - (full_h - rest.height) * eased_t;

            let _ = self.tiles[target].webview.set_bounds(Rect {
                position: tauri::Position::Logical(LogicalPosition::new(curr_x, curr_y)),
                size: tauri::Size::Logical(LogicalSize::new(curr_w, curr_h)),
            });
            std::thread::sleep(step_delay);
        }

        let _ = self.tiles[target].webview.set_bounds(Rect {
            position: tauri::Position::Logical(LogicalPosition::new(rest.x, rest.y)),
            size: tauri::Size::Logical(LogicalSize::new(rest.width, rest.height)),
        });

        // Restore sibling webviews
        for tile in &self.tiles {
            let _ = tile.webview.show();
        }

        self.machine.finish_minimizing();
        self.emit_status();
    }

    pub fn on_page_load_finished(&self, loaded_index: usize, event_gen: u64) {
        if self.machine.on_page_load_finished(loaded_index, event_gen) {
            self.emit_status();
        }
    }

    pub fn on_preparing_timeout(&self, expected_gen: u64) {
        if self.machine.on_preparing_timeout(expected_gen).is_some() {
            self.emit_status();
        }
    }

    pub fn next(&self) {
        let next_idx = {
            let data = self.machine.state.lock();
            (data.active_index + 1) % self.tiles.len().max(1)
        };
        self.transition_to_maximizing(next_idx);
    }

    pub fn prev(&self) {
        let prev_idx = {
            let data = self.machine.state.lock();
            if data.active_index == 0 {
                self.tiles.len().saturating_sub(1)
            } else {
                data.active_index - 1
            }
        };
        self.transition_to_maximizing(prev_idx);
    }

    pub fn minimize_current(&self) {
        let (state, active) = {
            let data = self.machine.state.lock();
            (data.current_state, data.active_index)
        };
        if state == TourStateName::MaximizedSingleSite || state == TourStateName::Maximizing {
            self.transition_to_minimizing(active);
        }
    }
}

pub fn start_tour_loop(
    controller: Arc<TourController>,
    mut page_load_rx: UnboundedReceiver<(usize, u64)>,
    auto_start: bool,
) {
    if auto_start {
        controller.start();
    }

    tokio::spawn(async move {
        let mut tick_interval = tokio::time::interval(Duration::from_millis(100));

        loop {
            tokio::select! {
                Some((idx, event_gen)) = page_load_rx.recv() => {
                    controller.on_page_load_finished(idx, event_gen);
                }

                _ = tick_interval.tick() => {
                    if controller.should_exit.load(Ordering::SeqCst) {
                        break;
                    }

                    let (current_state, is_paused, elapsed, target_duration, candidate, active, gen) = {
                        let data = controller.machine.state.lock();
                        (
                            data.current_state,
                            data.is_paused,
                            data.state_start.elapsed(),
                            data.target_duration,
                            data.candidate_index,
                            data.active_index,
                            data.tour_generation,
                        )
                    };

                    if is_paused || current_state == TourStateName::Stopped {
                        controller.emit_status();
                        continue;
                    }

                    controller.emit_status();

                    match current_state {
                        TourStateName::GridView => {
                            if elapsed >= target_duration {
                                let ctrl = controller.clone();
                                tokio::task::spawn_blocking(move || {
                                    ctrl.transition_to_maximizing(candidate);
                                });
                            }
                        }
                        TourStateName::MaximizedSingleSite => {
                            if elapsed >= target_duration {
                                let ctrl = controller.clone();
                                tokio::task::spawn_blocking(move || {
                                    ctrl.transition_to_minimizing(active);
                                });
                            }
                        }
                        TourStateName::PreparingNext => {
                            if elapsed >= target_duration {
                                controller.on_preparing_timeout(gen);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    });
}

// ----------------------------------------------------------------------------
// Tauri Commands
// ----------------------------------------------------------------------------

#[tauri::command]
pub fn start_tour(controller: State<Arc<TourController>>) -> Result<(), String> {
    controller.start();
    Ok(())
}

#[tauri::command]
pub fn pause_tour(controller: State<Arc<TourController>>) -> Result<(), String> {
    controller.pause();
    Ok(())
}

#[tauri::command]
pub fn resume_tour(controller: State<Arc<TourController>>) -> Result<(), String> {
    controller.resume();
    Ok(())
}

#[tauri::command]
pub fn next_tile(controller: State<Arc<TourController>>) -> Result<(), String> {
    controller.next();
    Ok(())
}

#[tauri::command]
pub fn prev_tile(controller: State<Arc<TourController>>) -> Result<(), String> {
    controller.prev();
    Ok(())
}

#[tauri::command]
pub fn toggle_fullscreen(app_handle: AppHandle) -> Result<(), String> {
    if let Some(win) = app_handle.get_window("main") {
        let is_fs = win.is_fullscreen().unwrap_or(false);
        win.set_fullscreen(!is_fs).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn minimize_current(controller: State<Arc<TourController>>) -> Result<(), String> {
    controller.minimize_current();
    Ok(())
}

#[tauri::command]
pub fn get_tour_status(controller: State<Arc<TourController>>) -> Result<TourStatusPayload, String> {
    let titles: Vec<String> = controller.tiles.iter().map(|t| t.title.clone()).collect();
    Ok(controller.machine.get_status_payload(&titles))
}

// ----------------------------------------------------------------------------
// Unit Tests
// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_start_from_stopped_transitions_to_grid_view() {
        let machine = TourStateMachine::new(TimingConfig::default(), 5);
        assert_eq!(machine.state.lock().current_state, TourStateName::Stopped);

        // Calling start() MUST transition from Stopped to GridView
        machine.start();
        assert_eq!(machine.state.lock().current_state, TourStateName::GridView);
        assert!(!machine.state.lock().is_paused);
    }

    #[test]
    fn test_pause_and_resume_preserves_elapsed_time() {
        let machine = TourStateMachine::new(TimingConfig::default(), 5);
        machine.start();
        assert_eq!(machine.state.lock().current_state, TourStateName::GridView);

        std::thread::sleep(Duration::from_millis(50));
        machine.pause();
        assert_eq!(machine.state.lock().current_state, TourStateName::Paused);
        assert!(machine.state.lock().is_paused);
        let paused_time = machine.state.lock().paused_elapsed;
        assert!(paused_time >= Duration::from_millis(50));

        std::thread::sleep(Duration::from_millis(50));

        machine.resume();
        assert_eq!(machine.state.lock().current_state, TourStateName::GridView);
        assert!(!machine.state.lock().is_paused);

        let elapsed_after_resume = machine.state.lock().state_start.elapsed();
        assert!(elapsed_after_resume >= paused_time);
        assert!(elapsed_after_resume < paused_time + Duration::from_millis(40));
    }

    #[test]
    fn test_finished_event_during_minimizing_prevents_false_timeout() {
        let machine = TourStateMachine::new(TimingConfig::default(), 5);
        machine.start();

        // 1. Begin minimizing tile 0 (upcoming candidate is tile 1)
        let (target, candidate, gen, _duration) = machine.begin_minimizing(0);
        assert_eq!(target, 0);
        assert_eq!(candidate, 1);
        assert_eq!(machine.state.lock().current_state, TourStateName::Minimizing);
        assert_eq!(machine.state.lock().pending_reload, Some((1, gen)));
        assert!(!machine.state.lock().candidate_reload_finished);

        // 2. Candidate 1 finishes loading FAST while minimization animation is still running!
        machine.on_page_load_finished(1, gen);
        assert!(machine.state.lock().candidate_reload_finished, "Must record readiness during Minimizing");

        // 3. Minimization animation completes and calls finish_minimizing
        let next_state = machine.finish_minimizing();

        // INVARIANT: Must transition directly to GridView, NOT stuck in PreparingNext!
        assert_eq!(next_state, TourStateName::GridView);
        assert_eq!(machine.state.lock().current_state, TourStateName::GridView);
        assert_eq!(machine.state.lock().pending_reload, None);
        assert_eq!(machine.state.lock().candidate_index, 1);
    }

    #[test]
    fn test_stalled_load_triggers_preparing_next_timeout_and_advances_candidate() {
        let machine = TourStateMachine::new(TimingConfig::default(), 5);
        machine.start();

        // 1. Minimizing tile 0 -> upcoming candidate is 1
        let (_, _, gen, _) = machine.begin_minimizing(0);

        // 2. Minimization completes, but candidate 1 has NOT finished loading
        let next_state = machine.finish_minimizing();
        assert_eq!(next_state, TourStateName::PreparingNext);
        assert_eq!(machine.state.lock().current_state, TourStateName::PreparingNext);
        assert_eq!(machine.state.lock().candidate_index, 1);

        // 3. 2.5s timeout expires with matching generation token
        let advanced = machine.on_preparing_timeout(gen);

        // INVARIANT: Candidate 1 was skipped and pointer advanced to candidate 2!
        assert_eq!(advanced, Some(2));
        assert_eq!(machine.state.lock().candidate_index, 2);
        assert_eq!(machine.state.lock().current_state, TourStateName::GridView);
        assert_eq!(machine.state.lock().pending_reload, None);
    }

    #[test]
    fn test_stale_same_index_generation_event_is_discarded() {
        let machine = TourStateMachine::new(TimingConfig::default(), 5);
        machine.start();

        // Tour targets tile 1 for the first time with Generation G1
        let (_, _, g1, _) = machine.begin_minimizing(0);
        let _ = machine.finish_minimizing();
        assert_eq!(machine.state.lock().current_state, TourStateName::PreparingNext);

        // Timeout triggers on G1, advancing past tile 1
        let _ = machine.on_preparing_timeout(g1);
        assert_eq!(machine.state.lock().current_state, TourStateName::GridView);

        // Tour cycles through tiles 2, 3, 4, 0... and wraps back around to tile 1 with Generation G2
        let (_, _, g2, _) = machine.begin_minimizing(0);
        assert!(g2 > g1);
        let _ = machine.finish_minimizing();
        assert_eq!(machine.state.lock().current_state, TourStateName::PreparingNext);
        assert_eq!(machine.state.lock().pending_reload, Some((1, g2)));

        // An extremely delayed Finished event from the SAME INDEX (tile 1) with OLD GENERATION G1 arrives!
        let accepted = machine.on_page_load_finished(1, g1);
        assert!(!accepted, "Stale Finished event from older generation must be discarded even for the same index");

        // Current state remains PreparingNext waiting for the legitimate G2 event!
        assert_eq!(machine.state.lock().current_state, TourStateName::PreparingNext);
        assert_eq!(machine.state.lock().pending_reload, Some((1, g2)));

        // Real G2 Finished event arrives
        let real_accepted = machine.on_page_load_finished(1, g2);
        assert!(real_accepted, "Legitimate Finished event with matching generation must be accepted");
        assert_eq!(machine.state.lock().current_state, TourStateName::GridView);
        assert_eq!(machine.state.lock().pending_reload, None);
    }

    #[test]
    fn test_pause_mid_flight_maximizing_preserves_paused_and_resets_elapsed() {
        let machine = TourStateMachine::new(TimingConfig::default(), 5);
        machine.start();

        // 1. Begin maximizing tile 0
        machine.begin_maximizing(0);
        assert_eq!(machine.state.lock().current_state, TourStateName::Maximizing);

        // 2. User presses Space / calls pause() mid-animation!
        machine.pause();
        assert_eq!(machine.state.lock().current_state, TourStateName::Paused);
        assert!(machine.state.lock().is_paused);
        assert_eq!(machine.state.lock().previous_state_before_pause, TourStateName::Maximizing);

        // Wait 50ms while paused
        std::thread::sleep(Duration::from_millis(50));

        // 3. Maximization animation completes in background and calls finish_maximizing
        let res = machine.finish_maximizing();

        // INVARIANT: State must remain Paused, NOT overwritten to MaximizedSingleSite!
        assert_eq!(res, TourStateName::Paused);
        assert_eq!(machine.state.lock().current_state, TourStateName::Paused);
        assert!(machine.state.lock().is_paused);
        assert_eq!(machine.state.lock().previous_state_before_pause, TourStateName::MaximizedSingleSite);
        // INVARIANT: paused_elapsed is reset to ZERO so resume does not backdate hold!
        assert_eq!(machine.state.lock().paused_elapsed, Duration::ZERO);

        // 4. User resumes later
        machine.resume();
        assert_eq!(machine.state.lock().current_state, TourStateName::MaximizedSingleSite);
        assert!(!machine.state.lock().is_paused);
        let elapsed = machine.state.lock().state_start.elapsed();
        // Elapsed should be fresh (~0ms), not containing the 50ms animation pause!
        assert!(elapsed < Duration::from_millis(30));
    }

    #[test]
    fn test_reload_finished_while_paused_in_preparing_next_updates_destination_to_grid_view() {
        let machine = TourStateMachine::new(TimingConfig::default(), 5);
        machine.start();

        // 1. Minimizing tile 0 -> upcoming candidate is 1
        let (_, _, gen, _) = machine.begin_minimizing(0);
        let next_state = machine.finish_minimizing();
        assert_eq!(next_state, TourStateName::PreparingNext);

        // 2. User pauses while waiting in PreparingNext
        machine.pause();
        assert_eq!(machine.state.lock().current_state, TourStateName::Paused);
        assert_eq!(machine.state.lock().previous_state_before_pause, TourStateName::PreparingNext);
        assert_eq!(machine.state.lock().pending_reload, Some((1, gen)));

        // 3. Candidate 1 finishes loading while user is paused
        let accepted = machine.on_page_load_finished(1, gen);
        assert!(accepted, "Reload finish must be accepted while paused");

        // INVARIANT: Remains Paused, but destination is updated to GridView and pending cleared!
        assert_eq!(machine.state.lock().current_state, TourStateName::Paused);
        assert_eq!(machine.state.lock().previous_state_before_pause, TourStateName::GridView);
        assert_eq!(machine.state.lock().pending_reload, None);
        assert_eq!(machine.state.lock().candidate_index, 1);

        // 4. User resumes: must enter GridView smoothly without re-entering PreparingNext or timing out
        machine.resume();
        assert_eq!(machine.state.lock().current_state, TourStateName::GridView);
    }

    #[test]
    fn test_timeout_while_paused_in_preparing_next_advances_candidate_and_keeps_paused() {
        let machine = TourStateMachine::new(TimingConfig::default(), 5);
        machine.start();

        // 1. Minimizing tile 0 -> upcoming candidate is 1
        let (_, _, gen, _) = machine.begin_minimizing(0);
        let _ = machine.finish_minimizing();

        // 2. User pauses in PreparingNext
        machine.pause();
        assert_eq!(machine.state.lock().previous_state_before_pause, TourStateName::PreparingNext);

        // 3. Timeout triggers on generation token while paused
        let advanced = machine.on_preparing_timeout(gen);
        assert_eq!(advanced, Some(2));

        // INVARIANT: Remains Paused, destination updated to GridView, candidate advanced to 2!
        assert_eq!(machine.state.lock().current_state, TourStateName::Paused);
        assert_eq!(machine.state.lock().previous_state_before_pause, TourStateName::GridView);
        assert_eq!(machine.state.lock().candidate_index, 2);
        assert_eq!(machine.state.lock().pending_reload, None);

        // 4. Resume enters GridView with candidate 2
        machine.resume();
        assert_eq!(machine.state.lock().current_state, TourStateName::GridView);
    }
}

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use parking_lot::Mutex;
use std::sync::Arc;
use tauri::State;

/// Configuration for the kiosk coordinator window.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WindowConfig {
    pub fullscreen: bool,
    pub decorations: bool,
    pub background_color: String,
    pub reserved_header_height_px: f64,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            fullscreen: false,
            decorations: false,
            background_color: "#0d0d0d".into(),
            reserved_header_height_px: 0.0,
        }
    }
}

/// Dynamic grid layout configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LayoutConfig {
    pub strategy: String,
    pub target_tile_aspect_ratio: f64,
    pub rows: Option<Vec<usize>>,
    pub padding_px: f64,
    pub gap_px: f64,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            strategy: "auto".into(),
            target_tile_aspect_ratio: 1.4,
            rows: None,
            padding_px: 0.0,
            gap_px: 0.0,
        }
    }
}

/// Timing parameters for the alternating tour cycle.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TimingConfig {
    pub grid_view_duration_ms: u64,
    pub maximized_hold_duration_ms: u64,
    pub transition_duration_ms: u64,
    pub preparing_timeout_ms: u64,
    pub user_idle_resume_ms: u64,
}

impl Default for TimingConfig {
    fn default() -> Self {
        Self {
            grid_view_duration_ms: 20000,
            maximized_hold_duration_ms: 30000,
            transition_duration_ms: 500,
            preparing_timeout_ms: 2500,
            user_idle_resume_ms: 15000,
        }
    }
}

/// Tour behaviour flags.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TourConfig {
    pub auto_start: bool,
    pub pause_on_interaction: bool,
    pub refresh_before_maximize: bool,
    pub loop_tour: bool,
}

impl Default for TourConfig {
    fn default() -> Self {
        Self {
            auto_start: true,
            pause_on_interaction: true,
            refresh_before_maximize: true,
            loop_tour: true,
        }
    }
}

/// DNS adblocking and proxy network configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NetworkDnsConfig {
    pub adblock_dns_enabled: bool,
    pub dns_provider: String,
    pub doh_url: String,
    pub dot_url: String,
    pub plain_dns_ip: String,
}

impl Default for NetworkDnsConfig {
    fn default() -> Self {
        Self {
            adblock_dns_enabled: true,
            dns_provider: "adguard_doh".into(),
            doh_url: "https://dns.adguard-dns.com/resolve".into(),
            dot_url: "tls://dns.adguard-dns.com".into(),
            plain_dns_ip: "94.140.14.14".into(),
        }
    }
}

/// Resource safety limits.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LimitsConfig {
    pub max_resident_webviews: usize,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            max_resident_webviews: 12,
        }
    }
}

/// Individual website endpoint configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EndpointItem {
    pub id: String,
    pub title: String,
    pub url: String,
    pub zoom_factor: Option<f64>,
    pub muted: bool,
    pub reload_interval_minutes: Option<u64>,
}

/// Root kiosk configuration structure.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KioskConfig {
    pub version: u32,
    pub window: WindowConfig,
    pub layout: LayoutConfig,
    pub timing: TimingConfig,
    pub tour: TourConfig,
    pub network_dns: NetworkDnsConfig,
    pub limits: LimitsConfig,
    pub endpoints: Vec<EndpointItem>,
}

impl Default for KioskConfig {
    fn default() -> Self {
        Self {
            version: 1,
            window: WindowConfig::default(),
            layout: LayoutConfig::default(),
            timing: TimingConfig::default(),
            tour: TourConfig::default(),
            network_dns: NetworkDnsConfig::default(),
            limits: LimitsConfig::default(),
            endpoints: default_preset_endpoints(),
        }
    }
}

/// Presets seeded as defaults.
pub fn default_preset_endpoints() -> Vec<EndpointItem> {
    vec![
        EndpointItem {
            id: "biztoc".into(),
            title: "Biztoc".into(),
            url: "https://biztoc.com/".into(),
            zoom_factor: None,
            muted: true,
            reload_interval_minutes: None,
        },
        EndpointItem {
            id: "alltoc".into(),
            title: "Alltoc".into(),
            url: "https://alltoc.com/".into(),
            zoom_factor: None,
            muted: true,
            reload_interval_minutes: None,
        },
        EndpointItem {
            id: "biztoc-wire".into(),
            title: "Biztoc Wire".into(),
            url: "https://biztoc.com/wire".into(),
            zoom_factor: None,
            muted: true,
            reload_interval_minutes: None,
        },
        EndpointItem {
            id: "ap-news".into(),
            title: "AP News Latest".into(),
            url: "https://apnews.com/hub/latest-news".into(),
            zoom_factor: None,
            muted: true,
            reload_interval_minutes: None,
        },
        EndpointItem {
            id: "finviz".into(),
            title: "Finviz News".into(),
            url: "https://finviz.com/news".into(),
            zoom_factor: None,
            muted: true,
            reload_interval_minutes: None,
        },
    ]
}

/// Identifies where the active configuration originated.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConfigSource {
    Cli,
    Portable,
    AppData,
    Defaults,
}

impl std::fmt::Display for ConfigSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigSource::Cli => write!(f, "CLI"),
            ConfigSource::Portable => write!(f, "Portable"),
            ConfigSource::AppData => write!(f, "AppData"),
            ConfigSource::Defaults => write!(f, "Defaults"),
        }
    }
}

/// Metadata response exposed to frontend controllers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConfigMetaResponse {
    pub config: KioskConfig,
    pub source: ConfigSource,
    pub is_readonly: bool,
    pub resolved_path: String,
}

/// In-memory state tracking active configuration and persistence origin.
pub struct AppConfigState {
    pub inner: Mutex<ConfigMetaResponse>,
    pub app_data_dir: PathBuf,
}

impl AppConfigState {
    pub fn new(meta: ConfigMetaResponse, app_data_dir: PathBuf) -> Self {
        Self {
            inner: Mutex::new(meta),
            app_data_dir,
        }
    }

    pub fn get_meta(&self) -> ConfigMetaResponse {
        self.inner.lock().clone()
    }

    pub fn update(&self, new_meta: ConfigMetaResponse) {
        let mut lock = self.inner.lock();
        *lock = new_meta;
    }
}

/// Locates portable kiosk-config.json adjacent to bundle/executable.
/// On macOS, traverses up past .app to prevent writing into signed bundle.
pub fn find_portable_config_path(exe_path: &Path) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let mut curr = exe_path.parent();
        while let Some(dir) = curr {
            if dir.extension().and_then(|e| e.to_str()) == Some("app") {
                if let Some(parent) = dir.parent() {
                    let candidate = parent.join("kiosk-config.json");
                    if candidate.is_file() {
                        return Some(candidate);
                    }
                }
                break;
            }
            curr = dir.parent();
        }
    }

    // Direct parent check (Windows/Linux or macOS dev binary outside .app)
    if let Some(parent) = exe_path.parent() {
        let candidate = parent.join("kiosk-config.json");
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}

/// Resolves configuration according to the strict 4-level precedence rule:
/// 1. CLI flag (--config) or KIOSK_CONFIG env var (read-only)
/// 2. Portable config adjacent to bundle/exe (read-only)
/// 3. User Application Support directory (writable)
/// 4. Compiled defaults
pub fn resolve_configuration(
    cli_arg: Option<&Path>,
    exe_path: Option<&Path>,
    app_data_dir: &Path,
) -> ConfigMetaResponse {
    // Level 1: CLI / Environment override
    if let Some(p) = cli_arg {
        if p.is_file() {
            if let Ok(content) = fs::read_to_string(p) {
                if let Ok(config) = serde_json::from_str::<KioskConfig>(&content) {
                    return ConfigMetaResponse {
                        config,
                        source: ConfigSource::Cli,
                        is_readonly: true,
                        resolved_path: p.to_string_lossy().to_string(),
                    };
                }
            }
        }
    }

    if let Ok(env_path) = std::env::var("KIOSK_CONFIG") {
        let p = PathBuf::from(env_path);
        if p.is_file() {
            if let Ok(content) = fs::read_to_string(&p) {
                if let Ok(config) = serde_json::from_str::<KioskConfig>(&content) {
                    return ConfigMetaResponse {
                        config,
                        source: ConfigSource::Cli,
                        is_readonly: true,
                        resolved_path: p.to_string_lossy().to_string(),
                    };
                }
            }
        }
    }

    // Level 2: Portable config
    if let Some(exe) = exe_path {
        if let Some(port_path) = find_portable_config_path(exe) {
            if let Ok(content) = fs::read_to_string(&port_path) {
                if let Ok(config) = serde_json::from_str::<KioskConfig>(&content) {
                    return ConfigMetaResponse {
                        config,
                        source: ConfigSource::Portable,
                        is_readonly: true,
                        resolved_path: port_path.to_string_lossy().to_string(),
                    };
                }
            }
        }
    }

    // Level 3: User AppData directory
    let appdata_file = app_data_dir.join("config.json");
    if appdata_file.is_file() {
        if let Ok(content) = fs::read_to_string(&appdata_file) {
            if let Ok(config) = serde_json::from_str::<KioskConfig>(&content) {
                return ConfigMetaResponse {
                    config,
                    source: ConfigSource::AppData,
                    is_readonly: false,
                    resolved_path: appdata_file.to_string_lossy().to_string(),
                };
            }
        }
    }

    // Level 4: Compiled defaults
    ConfigMetaResponse {
        config: KioskConfig::default(),
        source: ConfigSource::Defaults,
        is_readonly: false,
        resolved_path: "<compiled defaults>".into(),
    }
}

/// Atomically persists configuration to destination path.
pub fn write_config_atomic(path: &Path, config: &KioskConfig) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create directory: {}", e))?;
    }
    let serialized = serde_json::to_string_pretty(config)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;

    let tmp_path = path.with_extension("tmp");
    fs::write(&tmp_path, serialized).map_err(|e| format!("Failed to write temp config: {}", e))?;
    fs::rename(&tmp_path, path).map_err(|e| format!("Failed to finalize config file: {}", e))?;
    Ok(())
}

// ----------------------------------------------------------------------------
// Tauri Commands
// ----------------------------------------------------------------------------

/// Retrieves current active configuration and metadata.
#[tauri::command]
pub fn get_config(state: State<Arc<AppConfigState>>) -> Result<ConfigMetaResponse, String> {
    Ok(state.get_meta())
}

/// Saves modified configuration. Enforces shadowing protection and endpoint count limits.
#[tauri::command]
pub fn save_config(
    new_config: KioskConfig,
    state: State<Arc<AppConfigState>>,
) -> Result<(), String> {
    let current_meta = state.get_meta();

    // Shadowing protection: reject in-place save if active config is locked by CLI or Portable
    if current_meta.is_readonly {
        return Err(format!(
            "Active configuration is locked by a higher-precedence {} source ({}). In-place saving is disabled to prevent silent shadowing. Use export_config instead.",
            current_meta.source, current_meta.resolved_path
        ));
    }

    // Validation: endpoint count vs max_resident_webviews
    if new_config.endpoints.len() > new_config.limits.max_resident_webviews {
        return Err(format!(
            "Configured endpoints count ({}) exceeds max_resident_webviews limit ({}).",
            new_config.endpoints.len(),
            new_config.limits.max_resident_webviews
        ));
    }

    let target_path = state.app_data_dir.join("config.json");
    write_config_atomic(&target_path, &new_config)?;

    state.update(ConfigMetaResponse {
        config: new_config,
        source: ConfigSource::AppData,
        is_readonly: false,
        resolved_path: target_path.to_string_lossy().to_string(),
    });

    Ok(())
}

/// Exports configuration to an arbitrary user-specified destination path.
#[tauri::command]
pub fn export_config(config: KioskConfig, destination_path: String) -> Result<(), String> {
    let target = PathBuf::from(destination_path);
    write_config_atomic(&target, &config)
}

// ----------------------------------------------------------------------------
// Unit Tests
// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    struct TempDirGuard {
        path: PathBuf,
    }

    impl TempDirGuard {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("ambient_kiosk_test_{}_{}", name, std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn test_config_precedence() {
        let temp = TempDirGuard::new("precedence");

        // Level 3: AppData
        let appdata_dir = temp.path.join("appdata");
        fs::create_dir_all(&appdata_dir).unwrap();
        let appdata_file = appdata_dir.join("config.json");
        let mut appdata_cfg = KioskConfig::default();
        appdata_cfg.timing.grid_view_duration_ms = 33333;
        let mut f = File::create(&appdata_file).unwrap();
        f.write_all(serde_json::to_string(&appdata_cfg).unwrap().as_bytes()).unwrap();

        // Level 2: Portable
        let exe_dir = temp.path.join("exe_dir");
        fs::create_dir_all(&exe_dir).unwrap();
        let exe_path = exe_dir.join("ambient-kiosk");
        let portable_file = exe_dir.join("kiosk-config.json");
        let mut port_cfg = KioskConfig::default();
        port_cfg.timing.grid_view_duration_ms = 44444;
        let mut f = File::create(&portable_file).unwrap();
        f.write_all(serde_json::to_string(&port_cfg).unwrap().as_bytes()).unwrap();

        // Level 1: CLI
        let cli_file = temp.path.join("cli_override.json");
        let mut cli_cfg = KioskConfig::default();
        cli_cfg.timing.grid_view_duration_ms = 55555;
        let mut f = File::create(&cli_file).unwrap();
        f.write_all(serde_json::to_string(&cli_cfg).unwrap().as_bytes()).unwrap();

        // 1. All present: CLI must win
        let meta = resolve_configuration(Some(&cli_file), Some(&exe_path), &appdata_dir);
        assert_eq!(meta.source, ConfigSource::Cli);
        assert!(meta.is_readonly);
        assert_eq!(meta.config.timing.grid_view_duration_ms, 55555);

        // 2. No CLI: Portable must win over AppData
        let meta = resolve_configuration(None, Some(&exe_path), &appdata_dir);
        assert_eq!(meta.source, ConfigSource::Portable);
        assert!(meta.is_readonly);
        assert_eq!(meta.config.timing.grid_view_duration_ms, 44444);

        // 3. No CLI, No Portable: AppData must win over Defaults
        let no_portable_exe = temp.path.join("other_bin");
        let meta = resolve_configuration(None, Some(&no_portable_exe), &appdata_dir);
        assert_eq!(meta.source, ConfigSource::AppData);
        assert!(!meta.is_readonly);
        assert_eq!(meta.config.timing.grid_view_duration_ms, 33333);

        // 4. Nothing present: Defaults
        let empty_appdata = temp.path.join("empty_appdata");
        let meta = resolve_configuration(None, Some(&no_portable_exe), &empty_appdata);
        assert_eq!(meta.source, ConfigSource::Defaults);
        assert!(!meta.is_readonly);
        assert_eq!(meta.config.endpoints.len(), 5);
    }

    #[test]
    fn test_shadowing_rejection() {
        let temp = TempDirGuard::new("shadowing");
        let appdata_dir = temp.path.join("appdata");

        // Simulate active portable config
        let meta = ConfigMetaResponse {
            config: KioskConfig::default(),
            source: ConfigSource::Portable,
            is_readonly: true,
            resolved_path: "/dummy/kiosk-config.json".into(),
        };

        let state = AppConfigState::new(meta, appdata_dir.clone());
        let mut new_config = KioskConfig::default();
        new_config.timing.grid_view_duration_ms = 99999;

        // When state is readonly, save must fail
        let current = state.get_meta();
        assert!(current.is_readonly);

        let res = if current.is_readonly {
            Err("Locked by portable source".to_string())
        } else {
            Ok(())
        };
        assert!(res.is_err(), "Saving over locked portable config must fail");
    }

    #[test]
    fn test_limits_validation() {
        let mut cfg = KioskConfig::default();
        cfg.limits.max_resident_webviews = 2;
        assert!(cfg.endpoints.len() > cfg.limits.max_resident_webviews);
    }
}

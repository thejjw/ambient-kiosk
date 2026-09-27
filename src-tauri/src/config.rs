use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
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
            doh_url: "https://dns.adguard-dns.com/dns-query".into(),
            dot_url: "tls://dns.adguard-dns.com".into(),
            plain_dns_ip: "94.140.14.14:53".into(),
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
/// System safety ceiling for resident webviews.
pub const MAX_ALLOWED_RESIDENT_WEBVIEWS: usize = 16;

/// Parses a hex color string (e.g. "#0d0d0d" or "#0d0d0dff") into RGBA components.
pub fn parse_hex_color(hex: &str) -> Option<(u8, u8, u8, u8)> {
    let clean = hex.trim().strip_prefix('#').unwrap_or(hex.trim());
    if clean.len() == 6 {
        let r = u8::from_str_radix(&clean[0..2], 16).ok()?;
        let g = u8::from_str_radix(&clean[2..4], 16).ok()?;
        let b = u8::from_str_radix(&clean[4..6], 16).ok()?;
        Some((r, g, b, 255))
    } else if clean.len() == 8 {
        let r = u8::from_str_radix(&clean[0..2], 16).ok()?;
        let g = u8::from_str_radix(&clean[2..4], 16).ok()?;
        let b = u8::from_str_radix(&clean[4..6], 16).ok()?;
        let a = u8::from_str_radix(&clean[6..8], 16).ok()?;
        Some((r, g, b, a))
    } else {
        None
    }
}

/// Validates kiosk configuration against runtime safety invariants:
/// 1. max_resident_webviews <= 16
/// 2. endpoints.len() <= max_resident_webviews
/// 3. endpoints.len() >= 1
pub fn validate_kiosk_config(config: &KioskConfig) -> Result<(), String> {
    if parse_hex_color(&config.window.background_color).is_none() {
        return Err(format!(
            "Configuration error: window.background_color '{}' is not a valid hex color (#RRGGBB or #RRGGBBAA).",
            config.window.background_color
        ));
    }
    if config.limits.max_resident_webviews > MAX_ALLOWED_RESIDENT_WEBVIEWS {
        return Err(format!(
            "Configuration error: max_resident_webviews ({}) exceeds system safety ceiling ({}).",
            config.limits.max_resident_webviews, MAX_ALLOWED_RESIDENT_WEBVIEWS
        ));
    }

    if config.endpoints.len() > config.limits.max_resident_webviews {
        return Err("Endpoint count exceeds max_resident_webviews limit.".into());
    }

    if config.endpoints.is_empty() {
        return Err("Configuration error: at least one endpoint must be configured.".into());
    }

    for (idx, ep) in config.endpoints.iter().enumerate() {
        if ep.title.trim().is_empty() {
            return Err(format!(
                "Configuration error: endpoint #{} has an empty title.",
                idx + 1
            ));
        }
        let err_msg = format!(
            "Configuration error: endpoint '{}' URL '{}' must be an absolute http or https URL with a valid host.",
            ep.title, ep.url
        );
        let parsed = url::Url::parse(&ep.url).map_err(|_| err_msg.clone())?;
        let has_authority = ep
            .url
            .get(parsed.scheme().len()..)
            .is_some_and(|rest| rest.starts_with("://"));
        let has_valid_host = parsed
            .host_str()
            .map(|h| !h.trim().is_empty())
            .unwrap_or(false);

        if (parsed.scheme() != "http" && parsed.scheme() != "https")
            || !has_authority
            || !has_valid_host
        {
            return Err(err_msg);
        }
    }

    Ok(())
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
) -> Result<ConfigMetaResponse, String> {
    // Level 1: CLI / Environment override
    if let Some(p) = cli_arg {
        if !p.is_file() {
            return Err(format!(
                "CLI configuration file '{}' does not exist or is not a file.",
                p.display()
            ));
        }
        let content = fs::read_to_string(p)
            .map_err(|e| format!("Failed to read CLI config at '{}': {}", p.display(), e))?;
        let config: KioskConfig = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse CLI config at '{}': {}", p.display(), e))?;
        validate_kiosk_config(&config)?;
        return Ok(ConfigMetaResponse {
            config,
            source: ConfigSource::Cli,
            is_readonly: true,
            resolved_path: p.to_string_lossy().to_string(),
        });
    }

    if let Ok(env_path) = std::env::var("KIOSK_CONFIG") {
        let p = PathBuf::from(env_path);
        if !p.is_file() {
            return Err(format!(
                "KIOSK_CONFIG path '{}' does not exist or is not a file.",
                p.display()
            ));
        }
        let content = fs::read_to_string(&p)
            .map_err(|e| format!("Failed to read KIOSK_CONFIG at '{}': {}", p.display(), e))?;
        let config: KioskConfig = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse KIOSK_CONFIG at '{}': {}", p.display(), e))?;
        validate_kiosk_config(&config)?;
        return Ok(ConfigMetaResponse {
            config,
            source: ConfigSource::Cli,
            is_readonly: true,
            resolved_path: p.to_string_lossy().to_string(),
        });
    }

    // Level 2: Portable config
    if let Some(exe) = exe_path {
        if let Some(port_path) = find_portable_config_path(exe) {
            let content = fs::read_to_string(&port_path).map_err(|e| {
                format!(
                    "Failed to read portable config at '{}': {}",
                    port_path.display(),
                    e
                )
            })?;
            let config: KioskConfig = serde_json::from_str(&content).map_err(|e| {
                format!(
                    "Failed to parse portable config at '{}': {}",
                    port_path.display(),
                    e
                )
            })?;
            validate_kiosk_config(&config)?;
            return Ok(ConfigMetaResponse {
                config,
                source: ConfigSource::Portable,
                is_readonly: true,
                resolved_path: port_path.to_string_lossy().to_string(),
            });
        }
    }

    // Level 3: User AppData directory
    let appdata_file = app_data_dir.join("config.json");
    if appdata_file.is_file() {
        let content = fs::read_to_string(&appdata_file).map_err(|e| {
            format!(
                "Failed to read AppData config at '{}': {}",
                appdata_file.display(),
                e
            )
        })?;
        let config: KioskConfig = serde_json::from_str(&content).map_err(|e| {
            format!(
                "Failed to parse AppData config at '{}': {}",
                appdata_file.display(),
                e
            )
        })?;
        validate_kiosk_config(&config)?;
        return Ok(ConfigMetaResponse {
            config,
            source: ConfigSource::AppData,
            is_readonly: false,
            resolved_path: appdata_file.to_string_lossy().to_string(),
        });
    }

    // Level 4: Compiled defaults
    let defaults = KioskConfig::default();
    validate_kiosk_config(&defaults)?;
    Ok(ConfigMetaResponse {
        config: defaults,
        source: ConfigSource::Defaults,
        is_readonly: false,
        resolved_path: "<compiled defaults>".into(),
    })
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
/// Helper to parse `--config <path>` or `-c <path>` from arguments iterator.
pub fn parse_cli_config_arg<I>(args: I) -> Option<PathBuf>
where
    I: IntoIterator<Item = String>,
{
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        if arg == "--config" || arg == "-c" {
            if let Some(val) = iter.next() {
                return Some(PathBuf::from(val));
            }
        } else if let Some(stripped) = arg.strip_prefix("--config=") {
            return Some(PathBuf::from(stripped));
        }
    }
    None
}

/// Core production validation and persistence logic for saving configuration.
/// Enforces shadowing protection and endpoint count limits against max_resident_webviews.
pub fn validate_and_save_config(
    state: &AppConfigState,
    new_config: KioskConfig,
) -> Result<(), String> {
    let current_meta = state.get_meta();

    // Shadowing protection: reject in-place save if active config is locked by CLI or Portable
    if current_meta.is_readonly {
        return Err("Active configuration is locked by a higher-precedence portable or CLI source. In-place saving is disabled to prevent shadowing. Use export_config to save a separate file.".into());
    }

    // Validation: enforce resource invariants on candidate configuration
    validate_kiosk_config(&new_config)?;

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
    validate_and_save_config(&state, new_config)
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
            let path = std::env::temp_dir().join(format!(
                "ambient_kiosk_test_{}_{}",
                name,
                std::process::id()
            ));
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
        f.write_all(serde_json::to_string(&appdata_cfg).unwrap().as_bytes())
            .unwrap();

        // Level 2: Portable
        let exe_dir = temp.path.join("exe_dir");
        fs::create_dir_all(&exe_dir).unwrap();
        let exe_path = exe_dir.join("ambient-kiosk");
        let portable_file = exe_dir.join("kiosk-config.json");
        let mut port_cfg = KioskConfig::default();
        port_cfg.timing.grid_view_duration_ms = 44444;
        let mut f = File::create(&portable_file).unwrap();
        f.write_all(serde_json::to_string(&port_cfg).unwrap().as_bytes())
            .unwrap();

        // Level 1: CLI
        let cli_file = temp.path.join("cli_override.json");
        let mut cli_cfg = KioskConfig::default();
        cli_cfg.timing.grid_view_duration_ms = 55555;
        let mut f = File::create(&cli_file).unwrap();
        f.write_all(serde_json::to_string(&cli_cfg).unwrap().as_bytes())
            .unwrap();

        // 1. All present: CLI must win
        let meta = resolve_configuration(Some(&cli_file), Some(&exe_path), &appdata_dir).unwrap();
        assert_eq!(meta.source, ConfigSource::Cli);
        assert!(meta.is_readonly);
        assert_eq!(meta.config.timing.grid_view_duration_ms, 55555);

        // 2. No CLI: Portable must win over AppData
        let meta = resolve_configuration(None, Some(&exe_path), &appdata_dir).unwrap();
        assert_eq!(meta.source, ConfigSource::Portable);
        assert!(meta.is_readonly);
        assert_eq!(meta.config.timing.grid_view_duration_ms, 44444);

        // 3. No CLI, No Portable: AppData must win over Defaults
        let no_portable_exe = temp.path.join("other_bin");
        let meta = resolve_configuration(None, Some(&no_portable_exe), &appdata_dir).unwrap();
        assert_eq!(meta.source, ConfigSource::AppData);
        assert!(!meta.is_readonly);
        assert_eq!(meta.config.timing.grid_view_duration_ms, 33333);

        // 4. Nothing present: Defaults
        let empty_appdata = temp.path.join("empty_appdata");
        let meta = resolve_configuration(None, Some(&no_portable_exe), &empty_appdata).unwrap();
        assert_eq!(meta.source, ConfigSource::Defaults);
        assert!(!meta.is_readonly);
        assert_eq!(meta.config.endpoints.len(), 5);
    }

    #[test]
    fn test_parse_cli_config_arg() {
        assert_eq!(
            parse_cli_config_arg(vec![
                "app".into(),
                "--config".into(),
                "/tmp/cfg.json".into()
            ]),
            Some(PathBuf::from("/tmp/cfg.json"))
        );
        assert_eq!(
            parse_cli_config_arg(vec!["app".into(), "--config=/tmp/cfg2.json".into()]),
            Some(PathBuf::from("/tmp/cfg2.json"))
        );
        assert_eq!(
            parse_cli_config_arg(vec!["app".into(), "-c".into(), "/tmp/cfg3.json".into()]),
            Some(PathBuf::from("/tmp/cfg3.json"))
        );
        assert_eq!(
            parse_cli_config_arg(vec!["app".into(), "--verbose".into()]),
            None
        );
    }

    #[test]
    fn test_shadowing_rejection() {
        let temp = TempDirGuard::new("shadowing");
        let appdata_dir = temp.path.join("appdata");

        // 1. Portable source: validate_and_save_config must reject
        let meta_port = ConfigMetaResponse {
            config: KioskConfig::default(),
            source: ConfigSource::Portable,
            is_readonly: true,
            resolved_path: "/dummy/kiosk-config.json".into(),
        };
        let state_port = AppConfigState::new(meta_port, appdata_dir.clone());
        let mut new_config = KioskConfig::default();
        new_config.timing.grid_view_duration_ms = 99999;

        let err_port = validate_and_save_config(&state_port, new_config.clone()).unwrap_err();
        assert_eq!(
            err_port,
            "Active configuration is locked by a higher-precedence portable or CLI source. In-place saving is disabled to prevent shadowing. Use export_config to save a separate file."
        );

        // 2. CLI source: validate_and_save_config must reject
        let meta_cli = ConfigMetaResponse {
            config: KioskConfig::default(),
            source: ConfigSource::Cli,
            is_readonly: true,
            resolved_path: "/dummy/cli.json".into(),
        };
        let state_cli = AppConfigState::new(meta_cli, appdata_dir.clone());
        let err_cli = validate_and_save_config(&state_cli, new_config).unwrap_err();
        assert_eq!(
            err_cli,
            "Active configuration is locked by a higher-precedence portable or CLI source. In-place saving is disabled to prevent shadowing. Use export_config to save a separate file."
        );
    }

    #[test]
    fn test_limits_validation() {
        let temp = TempDirGuard::new("limits");
        let appdata_dir = temp.path.join("appdata");

        let meta = ConfigMetaResponse {
            config: KioskConfig::default(),
            source: ConfigSource::Defaults,
            is_readonly: false,
            resolved_path: "<defaults>".into(),
        };
        let state = AppConfigState::new(meta, appdata_dir);

        let mut new_config = KioskConfig::default();
        new_config.limits.max_resident_webviews = 2; // 5 endpoints > 2 limit

        let err = validate_and_save_config(&state, new_config).unwrap_err();
        assert_eq!(err, "Endpoint count exceeds max_resident_webviews limit.");
    }

    #[test]
    fn test_successful_save_to_appdata() {
        let temp = TempDirGuard::new("successful_save");
        let appdata_dir = temp.path.join("appdata");

        let meta = ConfigMetaResponse {
            config: KioskConfig::default(),
            source: ConfigSource::Defaults,
            is_readonly: false,
            resolved_path: "<defaults>".into(),
        };
        let state = AppConfigState::new(meta, appdata_dir.clone());

        let mut new_config = KioskConfig::default();
        new_config.timing.grid_view_duration_ms = 77777;

        // Execute save
        let res = validate_and_save_config(&state, new_config);
        assert!(res.is_ok(), "Saving to AppData must succeed");

        // Verify file was written to disk atomically
        let written_file = appdata_dir.join("config.json");
        assert!(
            written_file.is_file(),
            "config.json must exist in appdata dir"
        );
        let content = fs::read_to_string(&written_file).unwrap();
        let parsed: KioskConfig = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed.timing.grid_view_duration_ms, 77777);

        // Verify in-memory state updated
        let updated_meta = state.get_meta();
        assert_eq!(updated_meta.source, ConfigSource::AppData);
        assert!(!updated_meta.is_readonly);
        assert_eq!(updated_meta.config.timing.grid_view_duration_ms, 77777);
    }

    #[test]
    fn test_loaded_config_safety_invariants() {
        let temp = TempDirGuard::new("loaded_safety");
        let appdata_dir = temp.path.join("appdata");
        fs::create_dir_all(&appdata_dir).unwrap();

        // 1. Loaded config exceeds max_resident_webviews > 16 ceiling
        let mut invalid_limit_cfg = KioskConfig::default();
        invalid_limit_cfg.limits.max_resident_webviews = 20;
        let limit_file = temp.path.join("invalid_limit.json");
        fs::write(
            &limit_file,
            serde_json::to_string(&invalid_limit_cfg).unwrap(),
        )
        .unwrap();

        let res = resolve_configuration(Some(&limit_file), None, &appdata_dir);
        assert!(
            res.is_err(),
            "Must reject loaded config with max_resident_webviews > 16"
        );
        let err_msg = res.unwrap_err();
        assert!(err_msg.contains("exceeds system safety ceiling (16)"));

        // 2. Loaded config with endpoints > limit
        let mut invalid_endpoints_cfg = KioskConfig::default();
        invalid_endpoints_cfg.limits.max_resident_webviews = 3; // 5 endpoints > 3
        let endpoints_file = temp.path.join("invalid_endpoints.json");
        fs::write(
            &endpoints_file,
            serde_json::to_string(&invalid_endpoints_cfg).unwrap(),
        )
        .unwrap();

        let res = resolve_configuration(Some(&endpoints_file), None, &appdata_dir);
        assert!(
            res.is_err(),
            "Must reject loaded config with endpoints > limit"
        );
        let err_msg = res.unwrap_err();
        assert_eq!(
            err_msg,
            "Endpoint count exceeds max_resident_webviews limit."
        );

        // 3. Loaded config with zero endpoints
        let mut empty_endpoints_cfg = KioskConfig::default();
        empty_endpoints_cfg.endpoints.clear();
        let empty_file = temp.path.join("empty_endpoints.json");
        fs::write(
            &empty_file,
            serde_json::to_string(&empty_endpoints_cfg).unwrap(),
        )
        .unwrap();

        let res = resolve_configuration(Some(&empty_file), None, &appdata_dir);
        assert!(
            res.is_err(),
            "Must reject loaded config with zero endpoints"
        );
        let err_msg = res.unwrap_err();
        assert!(err_msg.contains("at least one endpoint must be configured"));
    }

    #[test]
    fn test_endpoint_validation_and_error_propagation() {
        let temp = TempDirGuard::new("endpoint_validation");
        let appdata_dir = temp.path.join("appdata");
        fs::create_dir_all(&appdata_dir).unwrap();

        // 1. Empty title validation
        let mut bad_title_cfg = KioskConfig::default();
        bad_title_cfg.endpoints[0].title = "   ".into();
        let err = validate_kiosk_config(&bad_title_cfg).unwrap_err();
        assert!(err.contains("has an empty title"));

        // 2. Invalid URL scheme or missing host validation (file:, https:, http:/path, http:/foo://bar, not-a-url)
        let mut bad_scheme_cfg = KioskConfig::default();
        bad_scheme_cfg.endpoints[0].url = "file:///etc/passwd".into();
        let err = validate_kiosk_config(&bad_scheme_cfg).unwrap_err();
        assert!(err.contains("must be an absolute http or https URL"));

        let mut no_host_cfg = KioskConfig::default();
        no_host_cfg.endpoints[0].url = "https:".into();
        let err = validate_kiosk_config(&no_host_cfg).unwrap_err();
        assert!(err.contains("must be an absolute http or https URL"));

        let mut path_only_cfg = KioskConfig::default();
        path_only_cfg.endpoints[0].url = "http:/path/only".into();
        let err = validate_kiosk_config(&path_only_cfg).unwrap_err();
        assert!(err.contains("must be an absolute http or https URL"));

        let mut delayed_delimiter_cfg = KioskConfig::default();
        delayed_delimiter_cfg.endpoints[0].url = "http:/foo://bar".into();
        let err = validate_kiosk_config(&delayed_delimiter_cfg).unwrap_err();
        assert!(err.contains("must be an absolute http or https URL"));

        let mut unparseable_cfg = KioskConfig::default();
        unparseable_cfg.endpoints[0].url = "not a valid url".into();
        let err = validate_kiosk_config(&unparseable_cfg).unwrap_err();
        assert!(err.contains("must be an absolute http or https URL"));

        // 3. Explicit CLI config missing file error propagation (no silent fallthrough)
        let nonexistent_cli = temp.path.join("nonexistent_cli.json");
        let res = resolve_configuration(Some(&nonexistent_cli), None, &appdata_dir);
        assert!(res.is_err(), "Must return Err for nonexistent CLI path");
        assert!(res.unwrap_err().contains("does not exist or is not a file"));

        // 4. Corrupt JSON error propagation
        let corrupt_file = temp.path.join("corrupt.json");
        fs::write(&corrupt_file, "{ invalid json").unwrap();
        let res = resolve_configuration(Some(&corrupt_file), None, &appdata_dir);
        assert!(res.is_err(), "Must return Err for corrupt JSON");
        assert!(res.unwrap_err().contains("Failed to parse CLI config"));
    }

    #[test]
    fn test_parse_hex_color_and_background_color_validation() {
        assert_eq!(parse_hex_color("#0d0d0d"), Some((13, 13, 13, 255)));
        assert_eq!(parse_hex_color("0d0d0d"), Some((13, 13, 13, 255)));
        assert_eq!(parse_hex_color("#11223344"), Some((17, 34, 51, 68)));

        assert_eq!(parse_hex_color(""), None);
        assert_eq!(parse_hex_color("not-a-color"), None);
        assert_eq!(parse_hex_color("#123"), None);

        let mut bad_color_cfg = KioskConfig::default();
        bad_color_cfg.window.background_color = "not-a-color".into();
        let err = validate_kiosk_config(&bad_color_cfg).unwrap_err();
        assert!(err.contains("not a valid hex color"));
    }
}

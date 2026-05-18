use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::ModexResult;

pub const APP_NAME: &str = "Modex";
pub const CONFIG_VERSION: u8 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppIdentity {
    pub name: String,
    pub codex_home: PathBuf,
    #[serde(default)]
    pub monitor: bool,
    #[serde(default)]
    pub workspace_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(default = "default_version")]
    pub version: u8,
    #[serde(default = "default_codex_binary")]
    pub codex_binary: String,
    #[serde(default = "default_app_name")]
    pub app_name: String,
    #[serde(default = "default_poll_seconds")]
    pub poll_seconds: u64,
    #[serde(default = "default_source_home")]
    pub source_home: PathBuf,
    #[serde(default)]
    pub has_completed_setup: bool,
    #[serde(default)]
    pub current_identity_name: Option<String>,
    #[serde(default)]
    pub daily_wake: DailyWakeSettings,
    #[serde(default)]
    pub identities: Vec<AppIdentity>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyWakeSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_daily_wake_time")]
    pub time: String,
    #[serde(default = "default_daily_wake_message")]
    pub message: String,
    #[serde(default = "default_daily_wake_primary_threshold")]
    pub skip_if_primary_used_above_percent: u8,
    #[serde(default = "default_daily_wake_weekly_remaining_threshold")]
    pub skip_if_weekly_remaining_below_percent: u8,
    #[serde(default = "default_daily_wake_max_primary_delta")]
    pub max_primary_delta_percent: u8,
    #[serde(default)]
    pub last_run_date: Option<String>,
}

impl Default for DailyWakeSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            time: default_daily_wake_time(),
            message: default_daily_wake_message(),
            skip_if_primary_used_above_percent: default_daily_wake_primary_threshold(),
            skip_if_weekly_remaining_below_percent: default_daily_wake_weekly_remaining_threshold(),
            max_primary_delta_percent: default_daily_wake_max_primary_delta(),
            last_run_date: None,
        }
    }
}

impl AppSettings {
    pub fn default_for_home(home: PathBuf) -> Self {
        Self {
            version: CONFIG_VERSION,
            codex_binary: default_codex_binary(),
            app_name: default_app_name(),
            poll_seconds: default_poll_seconds(),
            source_home: home.join(".codex"),
            has_completed_setup: false,
            current_identity_name: None,
            daily_wake: DailyWakeSettings::default(),
            identities: Vec::new(),
        }
    }
}

pub fn load_app_settings_from_path(path: &Path) -> ModexResult<AppSettings> {
    let raw = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&raw)?)
}

pub fn save_app_settings_to_path(settings: &AppSettings, path: &Path) -> ModexResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        path,
        format!("{}\n", serde_json::to_string_pretty(settings)?),
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

pub fn default_app_config_path() -> PathBuf {
    if cfg!(target_os = "windows") {
        dirs::config_dir()
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")))
            .join(APP_NAME)
            .join("config.json")
    } else {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Library")
            .join("Application Support")
            .join(APP_NAME)
            .join("config.json")
    }
}

fn default_version() -> u8 {
    CONFIG_VERSION
}

fn default_codex_binary() -> String {
    "codex".to_string()
}

fn default_app_name() -> String {
    "Codex".to_string()
}

fn default_poll_seconds() -> u64 {
    60
}

fn default_daily_wake_time() -> String {
    "08:30".to_string()
}

fn default_daily_wake_message() -> String {
    "Good morning".to_string()
}

fn default_daily_wake_primary_threshold() -> u8 {
    3
}

fn default_daily_wake_weekly_remaining_threshold() -> u8 {
    20
}

fn default_daily_wake_max_primary_delta() -> u8 {
    3
}

fn default_source_home() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".codex")
}

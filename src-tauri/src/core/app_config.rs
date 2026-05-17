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
    pub identities: Vec<AppIdentity>,
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

fn default_source_home() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".codex")
}

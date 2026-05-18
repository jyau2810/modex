use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::app_config::{
    default_app_config_path, load_app_settings_from_path, save_app_settings_to_path, AppIdentity,
    AppSettings, DailyWakeSettings,
};
use super::auth::{
    auth_identity_display_name, auth_identity_match_key, auth_plan_type, has_local_auth,
    unique_identity_name,
};
use super::codex::{activate_codex_app, open_codex_app, read_quota_snapshot, run_login};
use super::identity_home::{default_new_identity, random_digits};
use super::quota::{quota_display, QuotaDisplay, QuotaSnapshot};
use super::{ModexError, ModexResult};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityView {
    pub name: String,
    pub codex_home: String,
    pub monitor: bool,
    pub workspace_id: Option<String>,
    pub logged_in: bool,
    pub login_expired: bool,
    pub is_current: bool,
    pub quota: QuotaDisplay,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppViewState {
    pub codex_binary: String,
    pub app_name: String,
    pub poll_seconds: u64,
    pub source_home: String,
    pub has_completed_setup: bool,
    pub current_identity_name: Option<String>,
    pub daily_wake: DailyWakeSettings,
    #[serde(default)]
    pub is_refreshing: bool,
    pub identities: Vec<IdentityView>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
    pub codex_binary: Option<String>,
    pub app_name: Option<String>,
    pub poll_seconds: Option<u64>,
    pub source_home: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionResult {
    pub ok: bool,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportIdentityResult {
    pub ok: bool,
    pub message: String,
    pub identity: Option<IdentityView>,
    pub imported: bool,
}

pub struct AppEngine {
    settings: AppSettings,
    config_path: PathBuf,
    snapshots: HashMap<String, QuotaSnapshot>,
    errors: HashMap<String, String>,
    expired_identity_names: HashSet<String>,
}

impl AppEngine {
    pub fn load() -> ModexResult<Self> {
        let config_path = default_app_config_path();
        if config_path.exists() {
            let settings = load_app_settings_from_path(&config_path)?;
            Ok(Self::new(settings, config_path))
        } else {
            let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
            Ok(Self::new(AppSettings::default_for_home(home), config_path))
        }
    }

    pub fn new(settings: AppSettings, config_path: PathBuf) -> Self {
        let mut engine = Self {
            settings,
            config_path,
            snapshots: HashMap::new(),
            errors: HashMap::new(),
            expired_identity_names: HashSet::new(),
        };
        engine.refresh_identity_names_from_auth();
        engine
    }

    pub fn settings(&self) -> &AppSettings {
        &self.settings
    }

    pub fn app_state(&self) -> AppViewState {
        AppViewState {
            codex_binary: self.settings.codex_binary.clone(),
            app_name: self.settings.app_name.clone(),
            poll_seconds: self.settings.poll_seconds,
            source_home: self.settings.source_home.display().to_string(),
            has_completed_setup: self.settings.has_completed_setup,
            current_identity_name: self.settings.current_identity_name.clone(),
            daily_wake: self.settings.daily_wake.clone(),
            is_refreshing: false,
            identities: self
                .settings
                .identities
                .iter()
                .map(|identity| self.identity_view(identity))
                .collect(),
        }
    }

    pub fn add_identity(&mut self) -> ModexResult<IdentityView> {
        self.add_identity_with_digits(random_digits)
    }

    pub fn add_identity_with_digits(
        &mut self,
        random_digits: impl FnMut() -> String,
    ) -> ModexResult<IdentityView> {
        let home = managed_home_root(&self.settings);
        let mut identity = default_new_identity(&home, random_digits)?;
        if let Some(auth_name) = auth_identity_display_name(&identity.codex_home) {
            let name = unique_identity_name(
                &auth_name,
                self.settings
                    .identities
                    .iter()
                    .map(|identity| identity.name.as_str()),
            );
            identity.name = name;
        }
        self.settings.identities.push(identity.clone());
        self.settings.has_completed_setup = true;
        self.save()?;
        Ok(self.identity_view(&identity))
    }

    pub fn import_current_identity(&mut self) -> ModexResult<ImportIdentityResult> {
        self.import_current_identity_with_digits(random_digits)
    }

    pub fn import_current_identity_with_digits(
        &mut self,
        random_digits: impl FnMut() -> String,
    ) -> ModexResult<ImportIdentityResult> {
        let source_auth = self.settings.source_home.join("auth.json");
        if !source_auth.exists() {
            return Ok(ImportIdentityResult {
                ok: false,
                message: "当前 Codex 尚未登录，无法导入。".to_string(),
                identity: None,
                imported: false,
            });
        }

        if let Some(existing_name) = self.identity_name_for_auth_home(&self.settings.source_home) {
            self.settings.current_identity_name = Some(existing_name.clone());
            self.save()?;
            let identity = self.identity(&existing_name)?;
            return Ok(ImportIdentityResult {
                ok: true,
                message: format!("账号已存在，未重复导入：{existing_name}"),
                identity: Some(self.identity_view(&identity)),
                imported: false,
            });
        }

        let home = managed_home_root(&self.settings);
        let mut identity = default_new_identity(&home, random_digits)?;
        let imported_home = identity.codex_home.clone();
        if let Err(error) = copy_source_auth_to_identity_home(&source_auth, &imported_home) {
            let _ = fs::remove_dir_all(&imported_home);
            return Err(error);
        }

        let reserved_names = self
            .settings
            .identities
            .iter()
            .map(|identity| identity.name.as_str());
        identity.name = unique_identity_name(
            auth_identity_display_name(&imported_home)
                .as_deref()
                .unwrap_or("账号"),
            reserved_names,
        );

        self.settings.identities.push(identity.clone());
        self.settings.has_completed_setup = true;
        self.settings.current_identity_name = Some(identity.name.clone());
        self.save()?;
        let view = self.identity_view(&identity);
        Ok(ImportIdentityResult {
            ok: true,
            message: format!("已导入账号：{}", view.name),
            identity: Some(view),
            imported: true,
        })
    }

    pub fn delete_identity(&mut self, name: &str) -> ModexResult<ActionResult> {
        let Some(index) = self.identity_index(name) else {
            return Err(ModexError::from(format!("未知身份：{name}")));
        };
        self.settings.identities.remove(index);
        self.snapshots.remove(name);
        self.errors.remove(name);
        self.expired_identity_names.remove(name);
        if self.settings.current_identity_name.as_deref() == Some(name) {
            self.settings.current_identity_name = None;
        }
        if self.settings.identities.is_empty() {
            self.settings.has_completed_setup = false;
        }
        self.save()?;
        Ok(ActionResult {
            ok: true,
            message: format!("已删除账号：{name}"),
        })
    }

    pub fn update_settings(&mut self, patch: SettingsPatch) -> ModexResult<AppViewState> {
        if let Some(codex_binary) = clean_optional(patch.codex_binary) {
            self.settings.codex_binary = codex_binary;
        }
        if let Some(app_name) = clean_optional(patch.app_name) {
            self.settings.app_name = app_name;
        }
        if let Some(poll_seconds) = patch.poll_seconds {
            self.settings.poll_seconds = poll_seconds.max(10);
        }
        if let Some(source_home) = clean_optional(patch.source_home) {
            self.settings.source_home = PathBuf::from(source_home);
        }
        self.save()?;
        Ok(self.app_state())
    }

    pub fn update_daily_wake(&mut self, settings: DailyWakeSettings) -> ModexResult<AppViewState> {
        self.settings.daily_wake = sanitize_daily_wake_settings(settings);
        self.save()?;
        Ok(self.app_state())
    }

    pub fn set_daily_wake_last_run_date(&mut self, date: String) -> ModexResult<()> {
        self.settings.daily_wake.last_run_date = Some(date);
        self.save()
    }

    pub fn login_identity(&mut self, name: &str) -> ModexResult<ActionResult> {
        let identity = self.identity(name)?;
        run_login(&self.settings, &identity)?;
        Ok(ActionResult {
            ok: true,
            message: format!("已打开浏览器登录：{name}"),
        })
    }

    pub fn switch_identity(&mut self, name: &str) -> ModexResult<ActionResult> {
        let identity = self.identity(name)?;
        if self.settings.current_identity_name.as_deref() == Some(name) {
            activate_codex_app(&self.settings)?;
            return Ok(ActionResult {
                ok: true,
                message: "正在打开 Codex".to_string(),
            });
        }
        open_codex_app(&self.settings, &identity)?;
        self.set_current_identity(name)?;
        Ok(ActionResult {
            ok: true,
            message: format!("正在切换到账号：{name}"),
        })
    }

    pub fn refresh_identity(&mut self, name: &str) -> ModexResult<IdentityView> {
        let identity = self.identity(name)?;
        match read_quota_snapshot(&self.settings, &identity) {
            Ok(snapshot) => {
                self.set_snapshot(name, snapshot);
            }
            Err(error) => {
                self.set_error(name, error.to_string());
            }
        }
        self.identity(name)
            .map(|identity| self.identity_view(&identity))
    }

    pub fn refresh_all(&mut self) -> Vec<IdentityView> {
        let names = self
            .settings
            .identities
            .iter()
            .map(|identity| identity.name.clone())
            .collect::<Vec<_>>();
        for name in names {
            let _ = self.refresh_identity(&name);
        }
        self.app_state().identities
    }

    pub fn refresh_plan(&self) -> (AppSettings, Vec<AppIdentity>) {
        (self.settings.clone(), self.settings.identities.clone())
    }

    pub fn set_snapshot(&mut self, name: &str, snapshot: QuotaSnapshot) {
        self.snapshots.insert(name.to_string(), snapshot);
        self.errors.remove(name);
        self.expired_identity_names.remove(name);
    }

    pub fn set_error(&mut self, name: &str, error: String) {
        if is_login_expired_error(&error) {
            self.expired_identity_names.insert(name.to_string());
        } else {
            self.expired_identity_names.remove(name);
        }
        self.errors.insert(name.to_string(), error);
    }

    pub fn set_current_identity(&mut self, name: &str) -> ModexResult<()> {
        if self.identity_index(name).is_none() {
            return Err(ModexError::from(format!("未知身份：{name}")));
        }
        self.settings.current_identity_name = Some(name.to_string());
        self.save()
    }

    pub fn identity(&self, name: &str) -> ModexResult<AppIdentity> {
        self.settings
            .identities
            .iter()
            .find(|identity| identity.name == name)
            .cloned()
            .ok_or_else(|| ModexError::from(format!("未知身份：{name}")))
    }

    pub fn save(&self) -> ModexResult<()> {
        save_app_settings_to_path(&self.settings, &self.config_path)
    }

    pub fn sync_identity_names_from_auth(&mut self) -> ModexResult<bool> {
        let changed = self.refresh_identity_names_from_auth();
        if changed {
            self.save()?;
        }
        Ok(changed)
    }

    pub fn sync_current_identity_from_source_auth(&mut self) -> ModexResult<bool> {
        let current = self.identity_name_for_auth_home(&self.settings.source_home);
        if self.settings.current_identity_name == current {
            return Ok(false);
        }
        self.settings.current_identity_name = current;
        self.save()?;
        Ok(true)
    }

    fn identity_view(&self, identity: &AppIdentity) -> IdentityView {
        let error = self.errors.get(&identity.name).map(String::as_str);
        let quota = self
            .snapshots
            .get(&identity.name)
            .map(|snapshot| quota_display(Some(snapshot), error))
            .unwrap_or_else(|| {
                let mut display = quota_display(None, error);
                display.plan =
                    super::auth::plan_label(auth_plan_type(&identity.codex_home).as_deref());
                display
            });
        IdentityView {
            name: identity.name.clone(),
            codex_home: identity.codex_home.display().to_string(),
            monitor: identity.monitor,
            workspace_id: identity.workspace_id.clone(),
            logged_in: has_local_auth(&identity.codex_home)
                && !self.expired_identity_names.contains(&identity.name),
            login_expired: self.expired_identity_names.contains(&identity.name),
            is_current: self.settings.current_identity_name.as_deref()
                == Some(identity.name.as_str()),
            quota,
        }
    }

    fn refresh_identity_names_from_auth(&mut self) -> bool {
        let mut reserved: HashSet<String> = HashSet::new();
        let mut changed = false;
        for identity in &mut self.settings.identities {
            let base_name = auth_identity_display_name(&identity.codex_home)
                .unwrap_or_else(|| identity.name.clone());
            let name = unique_identity_name(&base_name, reserved.iter().map(String::as_str));
            reserved.insert(name.clone());
            if identity.name == name {
                continue;
            }
            let old_name = identity.name.clone();
            if self.settings.current_identity_name.as_deref() == Some(old_name.as_str()) {
                self.settings.current_identity_name = Some(name.clone());
            }
            if let Some(snapshot) = self.snapshots.remove(&old_name) {
                self.snapshots.insert(name.clone(), snapshot);
            }
            if let Some(error) = self.errors.remove(&old_name) {
                self.errors.insert(name.clone(), error);
            }
            if self.expired_identity_names.remove(&old_name) {
                self.expired_identity_names.insert(name.clone());
            }
            identity.name = name;
            changed = true;
        }
        changed
    }

    fn identity_index(&self, name: &str) -> Option<usize> {
        self.settings
            .identities
            .iter()
            .position(|identity| identity.name == name)
    }

    fn identity_name_for_auth_home(&self, codex_home: &std::path::Path) -> Option<String> {
        let key = auth_identity_match_key(codex_home)?;
        self.settings
            .identities
            .iter()
            .find(|identity| auth_identity_match_key(&identity.codex_home).as_deref() == Some(&key))
            .map(|identity| identity.name.clone())
    }
}

fn clean_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn sanitize_daily_wake_settings(mut settings: DailyWakeSettings) -> DailyWakeSettings {
    settings.time = sanitize_wake_time(&settings.time);
    settings.message = if settings.message.trim().is_empty() {
        DailyWakeSettings::default().message
    } else {
        settings.message.trim().chars().take(120).collect()
    };
    settings.skip_if_primary_used_above_percent =
        settings.skip_if_primary_used_above_percent.min(100);
    settings.skip_if_weekly_remaining_below_percent =
        settings.skip_if_weekly_remaining_below_percent.min(100);
    settings.max_primary_delta_percent = settings.max_primary_delta_percent.min(100);
    settings
}

fn sanitize_wake_time(value: &str) -> String {
    let mut parts = value.split(':');
    let hour = parts.next().and_then(|value| value.parse::<u8>().ok());
    let minute = parts.next().and_then(|value| value.parse::<u8>().ok());
    if parts.next().is_some() {
        return DailyWakeSettings::default().time;
    }
    match (hour, minute) {
        (Some(hour), Some(minute)) if hour < 24 && minute < 60 => {
            format!("{hour:02}:{minute:02}")
        }
        _ => DailyWakeSettings::default().time,
    }
}

fn managed_home_root(settings: &AppSettings) -> PathBuf {
    settings
        .source_home
        .parent()
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn is_login_expired_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    error.contains("账号缺少登录凭据")
        || lower.contains("missing login")
        || lower.contains("missing auth")
        || lower.contains("not logged in")
        || lower.contains("not authenticated")
        || lower.contains("unauthorized")
        || lower.contains("401")
        || (lower.contains("login") && (lower.contains("expired") || lower.contains("required")))
        || (lower.contains("auth") && (lower.contains("expired") || lower.contains("required")))
}

fn copy_source_auth_to_identity_home(
    source_auth: &std::path::Path,
    identity_home: &std::path::Path,
) -> ModexResult<()> {
    fs::create_dir_all(identity_home)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(identity_home, fs::Permissions::from_mode(0o700));
    }
    let target_auth = identity_home.join("auth.json");
    fs::copy(source_auth, &target_auth)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&target_auth, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

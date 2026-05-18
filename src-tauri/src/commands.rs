use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, State};

use crate::core::app_config::{AppIdentity, AppSettings};
use crate::core::codex::read_quota_snapshot;
use crate::core::engine::{
    ActionResult, AppEngine, AppViewState, IdentityView, ImportIdentityResult, SettingsPatch,
};
use crate::core::quota::QuotaSnapshot;

pub const STATE_UPDATED_EVENT: &str = "modex://state-updated";
pub const REFRESH_STARTED_EVENT: &str = "modex://refresh-started";
pub const REFRESH_FINISHED_EVENT: &str = "modex://refresh-finished";
const MIN_POLL_SECONDS: u64 = 10;

pub struct ModexState {
    pub engine: Mutex<AppEngine>,
    refreshing: Arc<AtomicBool>,
}

impl ModexState {
    pub fn new(engine: AppEngine) -> Self {
        Self {
            engine: Mutex::new(engine),
            refreshing: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn is_refreshing(&self) -> bool {
        self.refreshing.load(Ordering::Acquire)
    }

    pub fn try_begin_refresh(&self) -> Option<RefreshGuard> {
        self.refreshing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| RefreshGuard {
                refreshing: Arc::clone(&self.refreshing),
            })
    }
}

pub struct RefreshGuard {
    refreshing: Arc<AtomicBool>,
}

impl Drop for RefreshGuard {
    fn drop(&mut self) {
        self.refreshing.store(false, Ordering::Release);
    }
}

#[tauri::command]
pub fn get_app_state(state: State<'_, ModexState>) -> Result<AppViewState, String> {
    current_app_state(&state)
}

#[tauri::command]
pub fn add_identity(app: AppHandle, state: State<'_, ModexState>) -> Result<IdentityView, String> {
    let identity = with_engine(&state, |engine| engine.add_identity())?;
    refresh_tray(&app);
    Ok(identity)
}

#[tauri::command]
pub fn import_current_identity(
    app: AppHandle,
    state: State<'_, ModexState>,
) -> Result<ImportIdentityResult, String> {
    let result = with_engine(&state, |engine| engine.import_current_identity())?;
    refresh_tray(&app);
    Ok(result)
}

#[tauri::command]
pub fn delete_identity(
    app: AppHandle,
    state: State<'_, ModexState>,
    name: String,
) -> Result<ActionResult, String> {
    let result = with_engine(&state, |engine| engine.delete_identity(&name))?;
    refresh_tray(&app);
    Ok(result)
}

#[tauri::command]
pub fn switch_identity(
    app: AppHandle,
    state: State<'_, ModexState>,
    name: String,
) -> Result<ActionResult, String> {
    let result = with_engine(&state, |engine| engine.switch_identity(&name))?;
    refresh_tray(&app);
    Ok(result)
}

#[tauri::command]
pub fn login_identity(
    app: AppHandle,
    state: State<'_, ModexState>,
    name: String,
) -> Result<ActionResult, String> {
    let result = with_engine(&state, |engine| engine.login_identity(&name))?;
    refresh_tray(&app);
    Ok(result)
}

#[tauri::command]
pub fn refresh_identity(
    app: AppHandle,
    state: State<'_, ModexState>,
    name: String,
) -> Result<IdentityView, String> {
    let identity = match state.try_begin_refresh() {
        Some(guard) => {
            emit_refresh_started(&app);
            refresh_tray(&app);
            let result = refresh_identity_with_guard(&state, guard, &name);
            refresh_tray(&app);
            emit_refresh_finished(&app);
            let identity = result?;
            emit_state_updated(&app);
            identity
        }
        None => {
            refresh_tray(&app);
            current_app_state(&state)?
                .identities
                .into_iter()
                .find(|identity| identity.name == name)
                .ok_or_else(|| format!("未知身份：{name}"))?
        }
    };
    Ok(identity)
}

#[tauri::command]
pub fn refresh_all(
    app: AppHandle,
    state: State<'_, ModexState>,
) -> Result<Vec<IdentityView>, String> {
    let identities = match state.try_begin_refresh() {
        Some(guard) => {
            emit_refresh_started(&app);
            refresh_tray(&app);
            let result = refresh_all_with_guard(&state, guard);
            refresh_tray(&app);
            emit_refresh_finished(&app);
            let identities = result?;
            emit_state_updated(&app);
            identities
        }
        None => {
            refresh_tray(&app);
            current_app_state(&state)?.identities
        }
    };
    Ok(identities)
}

#[tauri::command]
pub fn update_settings(
    app: AppHandle,
    state: State<'_, ModexState>,
    settings_patch: SettingsPatch,
) -> Result<AppViewState, String> {
    let view = with_engine(&state, |engine| engine.update_settings(settings_patch))?;
    refresh_tray(&app);
    Ok(view)
}

#[tauri::command]
pub fn open_identity_directory(
    state: State<'_, ModexState>,
    name: String,
) -> Result<ActionResult, String> {
    let identity = with_engine(&state, |engine| engine.identity(&name))?;
    tauri_plugin_opener::open_path(identity.codex_home, None::<&str>)
        .map_err(|error| error.to_string())?;
    Ok(ActionResult {
        ok: true,
        message: "已打开账号目录".to_string(),
    })
}

#[tauri::command]
pub fn open_main_window(app: AppHandle) -> Result<(), String> {
    show_main_window(&app);
    Ok(())
}

pub fn show_main_window(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    {
        let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

pub fn refresh_all_if_idle(state: &ModexState) -> Result<Option<Vec<IdentityView>>, String> {
    refresh_all_if_idle_with_reader(state, read_quota_snapshot)
}

pub fn refresh_all_with_guard(
    state: &ModexState,
    guard: RefreshGuard,
) -> Result<Vec<IdentityView>, String> {
    refresh_all_with_reader(state, guard, read_quota_snapshot)
}

pub fn refresh_identity_with_guard(
    state: &ModexState,
    guard: RefreshGuard,
    name: &str,
) -> Result<IdentityView, String> {
    refresh_identity_with_reader(state, guard, name, read_quota_snapshot)
}

fn refresh_identity_with_reader(
    state: &ModexState,
    _guard: RefreshGuard,
    name: &str,
    reader: impl Fn(&AppSettings, &AppIdentity) -> crate::core::ModexResult<QuotaSnapshot>,
) -> Result<IdentityView, String> {
    let (settings, identity) = {
        let engine = state
            .engine
            .lock()
            .map_err(|_| "Modex state lock poisoned".to_string())?;
        let identity = engine.identity(name).map_err(|error| error.to_string())?;
        (engine.settings().clone(), identity)
    };

    let result = reader(&settings, &identity);

    let mut engine = state
        .engine
        .lock()
        .map_err(|_| "Modex state lock poisoned".to_string())?;
    if engine.identity(name).is_err() {
        return Err(format!("未知身份：{name}"));
    }
    match result {
        Ok(snapshot) => engine.set_snapshot(name, snapshot),
        Err(error) => engine.set_error(name, error.to_string()),
    }
    identity_view_from_engine(&engine, name)
}

fn refresh_all_with_reader(
    state: &ModexState,
    _guard: RefreshGuard,
    reader: impl Fn(&AppSettings, &AppIdentity) -> crate::core::ModexResult<QuotaSnapshot>,
) -> Result<Vec<IdentityView>, String> {
    let (settings, identities) = {
        let engine = state
            .engine
            .lock()
            .map_err(|_| "Modex state lock poisoned".to_string())?;
        engine.refresh_plan()
    };

    let mut results = Vec::with_capacity(identities.len());
    for identity in identities {
        let name = identity.name.clone();
        results.push((name, reader(&settings, &identity)));
    }

    let mut engine = state
        .engine
        .lock()
        .map_err(|_| "Modex state lock poisoned".to_string())?;
    for (name, result) in results {
        if engine.identity(&name).is_err() {
            continue;
        }
        match result {
            Ok(snapshot) => engine.set_snapshot(&name, snapshot),
            Err(error) => engine.set_error(&name, error.to_string()),
        }
    }
    Ok(engine.app_state().identities)
}

fn refresh_all_if_idle_with_reader(
    state: &ModexState,
    reader: impl Fn(&AppSettings, &AppIdentity) -> crate::core::ModexResult<QuotaSnapshot>,
) -> Result<Option<Vec<IdentityView>>, String> {
    let Some(guard) = state.try_begin_refresh() else {
        return Ok(None);
    };
    refresh_all_with_reader(state, guard, reader).map(Some)
}

pub fn emit_state_updated(app: &AppHandle) {
    let _ = app.emit(STATE_UPDATED_EVENT, ());
}

pub fn emit_refresh_started(app: &AppHandle) {
    let _ = app.emit(REFRESH_STARTED_EVENT, ());
}

pub fn emit_refresh_finished(app: &AppHandle) {
    let _ = app.emit(REFRESH_FINISHED_EVENT, ());
}

pub fn start_startup_refresh(app: AppHandle) {
    start_refresh_all_with_events(app, "startup");
}

pub fn start_refresh_all_with_events(app: AppHandle, source: &'static str) {
    std::thread::spawn(move || {
        let state = app.state::<ModexState>();
        let Some(guard) = state.try_begin_refresh() else {
            let _ = refresh_tray(&app);
            return;
        };

        emit_refresh_started(&app);
        let _ = refresh_tray(&app);
        let refreshed = match refresh_all_with_guard(&state, guard) {
            Ok(_) => true,
            Err(error) => {
                eprintln!("modex {source} refresh failed: {error}");
                false
            }
        };
        let _ = refresh_tray(&app);
        emit_refresh_finished(&app);
        if refreshed {
            emit_state_updated(&app);
        }
    });
}

pub fn start_background_monitor(app: AppHandle) {
    std::thread::spawn(move || loop {
        let poll_seconds = poll_seconds(&app);
        std::thread::sleep(Duration::from_secs(poll_seconds));

        let refreshed = {
            let state = app.state::<ModexState>();
            match refresh_all_if_idle(&state) {
                Ok(Some(_)) => true,
                Ok(None) => false,
                Err(error) => {
                    eprintln!("modex background refresh failed: {error}");
                    false
                }
            }
        };

        if refreshed {
            refresh_tray(&app);
            emit_state_updated(&app);
        }
    });
}

fn refresh_tray(app: &AppHandle) {
    let _ = crate::tray::refresh_menu(app);
}

fn poll_seconds(app: &AppHandle) -> u64 {
    let state = app.state::<ModexState>();
    state
        .engine
        .lock()
        .map(|engine| engine.settings().poll_seconds.max(MIN_POLL_SECONDS))
        .unwrap_or(MIN_POLL_SECONDS)
}

fn with_engine<T>(
    state: &State<'_, ModexState>,
    action: impl FnOnce(&mut AppEngine) -> crate::core::ModexResult<T>,
) -> Result<T, String> {
    let mut engine = state
        .engine
        .lock()
        .map_err(|_| "Modex state lock poisoned".to_string())?;
    action(&mut engine).map_err(|error| error.to_string())
}

fn current_app_state(state: &ModexState) -> Result<AppViewState, String> {
    let mut engine = state
        .engine
        .lock()
        .map_err(|_| "Modex state lock poisoned".to_string())?;
    engine
        .sync_identity_names_from_auth()
        .map_err(|error| error.to_string())?;
    let mut view = engine.app_state();
    view.is_refreshing = state.is_refreshing();
    Ok(view)
}

fn identity_view_from_engine(engine: &AppEngine, name: &str) -> Result<IdentityView, String> {
    engine
        .app_state()
        .identities
        .into_iter()
        .find(|identity| identity.name == name)
        .ok_or_else(|| format!("未知身份：{name}"))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::time::Duration;

    use crate::core::app_config::{AppIdentity, AppSettings};
    use crate::core::quota::QuotaSnapshot;
    use crate::core::ModexError;

    use super::*;

    #[test]
    fn refresh_all_releases_engine_lock_while_quota_reader_is_running() {
        let temp = assert_fs::TempDir::new().unwrap();
        let mut settings = AppSettings::default_for_home(temp.path().to_path_buf());
        settings.identities.push(AppIdentity {
            name: "Team".to_string(),
            codex_home: PathBuf::from("/tmp/team"),
            monitor: false,
            workspace_id: None,
        });
        let state = ModexState::new(AppEngine::new(settings, temp.path().join("config.json")));
        let guard = state.try_begin_refresh().unwrap();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (continue_tx, continue_rx) = mpsc::channel();

        std::thread::scope(|scope| {
            let state_ref = &state;
            let refresh = scope.spawn(move || {
                refresh_all_with_reader(state_ref, guard, move |_settings, _identity| {
                    entered_tx.send(()).unwrap();
                    continue_rx.recv().unwrap();
                    Err(ModexError::from("reader failed"))
                })
                .unwrap()
            });

            entered_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("quota reader should start");
            let state_snapshot = state
                .engine
                .try_lock()
                .expect("engine lock should remain readable during quota I/O")
                .app_state();
            assert_eq!(state_snapshot.identities.len(), 1);

            continue_tx.send(()).unwrap();
            let refreshed = refresh.join().unwrap();
            assert_eq!(refreshed[0].quota.status, "error");
        });
    }

    #[test]
    fn refresh_all_with_reader_applies_successful_snapshots() {
        let temp = assert_fs::TempDir::new().unwrap();
        let mut settings = AppSettings::default_for_home(temp.path().to_path_buf());
        settings.identities.push(AppIdentity {
            name: "Team".to_string(),
            codex_home: PathBuf::from("/tmp/team"),
            monitor: false,
            workspace_id: None,
        });
        let state = ModexState::new(AppEngine::new(settings, temp.path().join("config.json")));
        let guard = state.try_begin_refresh().unwrap();

        let refreshed = refresh_all_with_reader(&state, guard, |_settings, identity| {
            Ok(QuotaSnapshot {
                identity: identity.name.clone(),
                plan_type: Some("team".to_string()),
                primary: None,
                secondary: None,
                credits_has_credits: Some(true),
                credits_unlimited: Some(false),
                reached_type: None,
            })
        })
        .unwrap();

        assert_eq!(refreshed[0].quota.status, "available");
        assert_eq!(refreshed[0].quota.plan, "团队版");
    }

    #[test]
    fn refresh_identity_with_reader_applies_only_the_requested_snapshot() {
        let temp = assert_fs::TempDir::new().unwrap();
        let mut settings = AppSettings::default_for_home(temp.path().to_path_buf());
        settings.identities.push(AppIdentity {
            name: "Team".to_string(),
            codex_home: PathBuf::from("/tmp/team"),
            monitor: false,
            workspace_id: None,
        });
        settings.identities.push(AppIdentity {
            name: "Backup".to_string(),
            codex_home: PathBuf::from("/tmp/backup"),
            monitor: false,
            workspace_id: None,
        });
        let state = ModexState::new(AppEngine::new(settings, temp.path().join("config.json")));
        let guard = state.try_begin_refresh().unwrap();

        let refreshed =
            refresh_identity_with_reader(&state, guard, "Backup", |_settings, identity| {
                assert_eq!(identity.name, "Backup");
                Ok(QuotaSnapshot {
                    identity: identity.name.clone(),
                    plan_type: Some("team".to_string()),
                    primary: None,
                    secondary: None,
                    credits_has_credits: Some(true),
                    credits_unlimited: Some(false),
                    reached_type: None,
                })
            })
            .unwrap();

        assert_eq!(refreshed.name, "Backup");
        assert_eq!(refreshed.quota.status, "available");
        let app_state = state.engine.lock().unwrap().app_state();
        let team = app_state
            .identities
            .iter()
            .find(|identity| identity.name == "Team")
            .unwrap();
        let backup = app_state
            .identities
            .iter()
            .find(|identity| identity.name == "Backup")
            .unwrap();
        assert_eq!(team.quota.status, "unknown");
        assert_eq!(backup.quota.status, "available");
    }

    #[test]
    fn refresh_all_if_idle_with_reader_skips_when_refresh_is_already_running() {
        let temp = assert_fs::TempDir::new().unwrap();
        let state = ModexState::new(AppEngine::new(
            AppSettings::default_for_home(temp.path().to_path_buf()),
            temp.path().join("config.json"),
        ));
        let _guard = state.try_begin_refresh().unwrap();

        let refreshed = refresh_all_if_idle_with_reader(&state, |_settings, _identity| {
            panic!("quota reader should not run while refresh guard is held")
        })
        .unwrap();

        assert!(refreshed.is_none());
    }
}

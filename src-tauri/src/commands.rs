use std::process::Command;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, State};

use crate::core::app_config::{AppIdentity, AppSettings, DailyWakeSettings};
use crate::core::auth::{auth_plan_type, plan_label};
use crate::core::codex::read_quota_snapshot;
use crate::core::engine::{
    ActionResult, AppEngine, AppViewState, IdentityView, ImportIdentityResult, SettingsPatch,
};
use crate::core::quota::QuotaSnapshot;
use crate::core::wake::{
    append_wake_log_entry, default_wake_log_path, finalize_wake_quota_evidence,
    primary_delta_exceeds_limit, read_recent_wake_log_entries, run_wake_prompt,
    should_wake_identity, timestamp_millis, wake_quota_evidence, weekly_remaining_percent,
    WakeAuditEntry, WakeDecision, WakeSkipReason, WakeThresholds,
};
use crate::notifications::{
    refresh_notifications, send_notification, send_notifications, switch_failure_notification,
    switch_success_notification,
};

pub const STATE_UPDATED_EVENT: &str = "modex://state-updated";
pub const REFRESH_STARTED_EVENT: &str = "modex://refresh-started";
pub const REFRESH_FINISHED_EVENT: &str = "modex://refresh-finished";
pub const LOG_ENTRY_EVENT: &str = "modex://log-entry";
const MIN_POLL_SECONDS: u64 = 10;
const WAKE_CHECK_SECONDS: u64 = 30;
const WAKE_WINDOW_MINUTES: u16 = 5;
const WAKE_TIMEOUT_SECONDS: u64 = 60;
const WAKE_QUOTA_SETTLE_SECONDS: u64 = 120;
const RECENT_LOG_LIMIT: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WakeRunMode {
    Scheduled,
    Manual,
}

impl WakeRunMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Scheduled => "scheduled",
            Self::Manual => "manual",
        }
    }
}

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
pub async fn switch_identity(app: AppHandle, name: String) -> Result<ActionResult, String> {
    run_blocking(move || {
        let state = app.state::<ModexState>();
        let result = switch_identity_with_notifications(&app, state.inner(), &name)?;
        refresh_tray(&app);
        Ok(result)
    })
    .await
}

#[tauri::command]
pub async fn login_identity(app: AppHandle, name: String) -> Result<ActionResult, String> {
    run_blocking(move || {
        let state = app.state::<ModexState>();
        let result = with_engine_ref(state.inner(), |engine| engine.login_identity(&name))?;
        refresh_tray(&app);
        Ok(result)
    })
    .await
}

#[tauri::command]
pub async fn refresh_identity(app: AppHandle, name: String) -> Result<IdentityView, String> {
    run_blocking(move || {
        let state = app.state::<ModexState>();
        let identity = match state.try_begin_refresh() {
            Some(guard) => {
                let before = current_identity_views(state.inner())?;
                emit_refresh_started(&app);
                refresh_tray(&app);
                let result = refresh_identity_with_guard(state.inner(), guard, &name);
                refresh_tray(&app);
                emit_refresh_finished(&app);
                let identity = result?;
                send_refresh_notifications(&app, &before, std::slice::from_ref(&identity));
                emit_state_updated(&app);
                identity
            }
            None => {
                refresh_tray(&app);
                current_app_state(state.inner())?
                    .identities
                    .into_iter()
                    .find(|identity| identity.name == name)
                    .ok_or_else(|| format!("未知身份：{name}"))?
            }
        };
        Ok(identity)
    })
    .await
}

#[tauri::command]
pub async fn refresh_all(app: AppHandle) -> Result<Vec<IdentityView>, String> {
    run_blocking(move || {
        let state = app.state::<ModexState>();
        let identities = match state.try_begin_refresh() {
            Some(guard) => {
                let before = current_identity_views(state.inner())?;
                emit_refresh_started(&app);
                refresh_tray(&app);
                let result = refresh_all_with_guard(state.inner(), guard);
                refresh_tray(&app);
                emit_refresh_finished(&app);
                let identities = result?;
                send_refresh_notifications(&app, &before, &identities);
                emit_state_updated(&app);
                identities
            }
            None => {
                refresh_tray(&app);
                current_app_state(state.inner())?.identities
            }
        };
        Ok(identities)
    })
    .await
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
pub fn update_daily_wake_settings(
    app: AppHandle,
    state: State<'_, ModexState>,
    daily_wake: DailyWakeSettings,
) -> Result<AppViewState, String> {
    let view = with_engine(&state, |engine| engine.update_daily_wake(daily_wake))?;
    refresh_tray(&app);
    emit_state_updated(&app);
    Ok(view)
}

#[tauri::command]
pub fn get_recent_log_entries() -> Result<Vec<WakeAuditEntry>, String> {
    read_recent_wake_log_entries(&default_wake_log_path(), RECENT_LOG_LIMIT)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn run_daily_wake_now(app: AppHandle) -> Result<ActionResult, String> {
    run_blocking(move || {
        let today = local_date_and_time()
            .map(|(today, _)| today)
            .ok_or_else(|| "无法读取本地日期，测试唤醒未启动。".to_string())?;
        run_daily_wake_job(&app, &today, WakeRunMode::Manual, None)?;
        Ok(ActionResult {
            ok: true,
            message: "已完成一次测试唤醒，详情见日志。".to_string(),
        })
    })
    .await
}

#[tauri::command]
pub async fn open_identity_directory(app: AppHandle, name: String) -> Result<ActionResult, String> {
    run_blocking(move || {
        let state = app.state::<ModexState>();
        let identity = with_engine_ref(state.inner(), |engine| engine.identity(&name))?;
        tauri_plugin_opener::open_path(identity.codex_home, None::<&str>)
            .map_err(|error| error.to_string())?;
        Ok(ActionResult {
            ok: true,
            message: "已打开账号目录".to_string(),
        })
    })
    .await
}

#[tauri::command]
pub fn open_main_window(app: AppHandle) -> Result<(), String> {
    show_main_window(&app);
    Ok(())
}

pub fn switch_identity_with_notifications(
    app: &AppHandle,
    state: &ModexState,
    name: &str,
) -> Result<ActionResult, String> {
    let result = {
        let mut engine = state
            .engine
            .lock()
            .map_err(|_| "Modex state lock poisoned".to_string())?;
        let before = engine.settings().current_identity_name.clone();
        engine
            .switch_identity(name)
            .map_err(|error| error.to_string())
            .map(|result| {
                let after = engine.settings().current_identity_name.clone();
                let switched =
                    result.ok && before.as_deref() != Some(name) && after.as_deref() == Some(name);
                (result, switched)
            })
    };
    match result {
        Ok((result, switched)) => {
            if switched {
                send_notification(app, &switch_success_notification(name));
            }
            Ok(result)
        }
        Err(reason) => {
            send_notification(app, &switch_failure_notification(name, &reason));
            Err(reason)
        }
    }
}

pub fn show_main_window(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    {
        if show_main_window_uses_regular_activation_policy() {
            let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
        }
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[cfg(target_os = "macos")]
fn show_main_window_uses_regular_activation_policy() -> bool {
    false
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
    engine
        .sync_current_identity_from_source_auth()
        .map_err(|error| error.to_string())?;
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
    engine
        .sync_current_identity_from_source_auth()
        .map_err(|error| error.to_string())?;
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
        let before = current_identity_views(&state).unwrap_or_default();

        emit_refresh_started(&app);
        let _ = refresh_tray(&app);
        let refreshed = match refresh_all_with_guard(&state, guard) {
            Ok(identities) => {
                send_refresh_notifications(&app, &before, &identities);
                true
            }
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
            let before = current_identity_views(&state).unwrap_or_default();
            match refresh_all_if_idle(&state) {
                Ok(Some(identities)) => {
                    send_refresh_notifications(&app, &before, &identities);
                    true
                }
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

pub fn start_daily_wake_scheduler(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(WAKE_CHECK_SECONDS));
        let Some((today, current_time)) = local_date_and_time() else {
            continue;
        };
        let settings = {
            let state = app.state::<ModexState>();
            state
                .engine
                .lock()
                .map(|engine| engine.settings().daily_wake.clone())
                .unwrap_or_default()
        };
        let Some(scheduled_time) = pending_daily_wake_time(&settings, &today, &current_time) else {
            continue;
        };
        if let Err(error) =
            run_daily_wake_job(&app, &today, WakeRunMode::Scheduled, Some(&scheduled_time))
        {
            eprintln!("modex daily wake failed: {error}");
        }
    });
}

pub fn should_start_daily_wake(
    settings: &DailyWakeSettings,
    today: &str,
    current_time: &str,
) -> bool {
    pending_daily_wake_time(settings, today, current_time).is_some()
}

pub fn pending_daily_wake_time(
    settings: &DailyWakeSettings,
    today: &str,
    current_time: &str,
) -> Option<String> {
    if !settings.enabled {
        return None;
    }
    let Some(current) = minutes_since_midnight(current_time) else {
        return None;
    };
    daily_wake_times(settings)
        .into_iter()
        .find(|time| {
            let Some(scheduled) = minutes_since_midnight(time) else {
                return false;
            };
            current >= scheduled
                && current.saturating_sub(scheduled) <= WAKE_WINDOW_MINUTES
                && !settings
                    .last_run_slots
                    .iter()
                    .any(|slot| slot == &wake_slot_key(today, time))
        })
        .map(ToString::to_string)
}

fn run_daily_wake_job(
    app: &AppHandle,
    today: &str,
    mode: WakeRunMode,
    scheduled_time: Option<&str>,
) -> Result<(), String> {
    let state = app.state::<ModexState>();
    let Some(guard) = state.try_begin_refresh() else {
        if mode == WakeRunMode::Manual {
            return Err("当前正在刷新或唤醒，请稍后再试。".to_string());
        }
        return Ok(());
    };
    let (settings, identities, mut wake_settings) = {
        let mut engine = state
            .engine
            .lock()
            .map_err(|_| "Modex state lock poisoned".to_string())?;
        let settings = engine.settings().clone();
        if mode == WakeRunMode::Scheduled
            && (!settings.daily_wake.enabled
                || scheduled_time.is_none_or(|time| {
                    settings
                        .daily_wake
                        .last_run_slots
                        .iter()
                        .any(|slot| slot == &wake_slot_key(today, time))
                }))
        {
            return Ok(());
        }
        if mode == WakeRunMode::Scheduled {
            if let Some(time) = scheduled_time {
                engine
                    .set_daily_wake_last_run_slot(today.to_string(), time.to_string())
                    .map_err(|error| error.to_string())?;
            }
        }
        (
            settings.clone(),
            settings.identities.clone(),
            settings.daily_wake.clone(),
        )
    };
    wake_settings.last_run_date = None;

    emit_refresh_started(app);
    let run_id = format!("wake-{}-{today}-{}", mode.as_str(), timestamp_millis());
    let mut circuit_breaker = false;
    for identity in identities {
        if circuit_breaker {
            break;
        }
        if is_known_non_team_identity(&identity) {
            continue;
        }
        let before_refresh = read_quota_snapshot(&settings, &identity);
        let identity_view = {
            let mut engine = state
                .engine
                .lock()
                .map_err(|_| "Modex state lock poisoned".to_string())?;
            match before_refresh {
                Ok(snapshot) => engine.set_snapshot(&identity.name, snapshot),
                Err(error) => engine.set_error(&identity.name, error.to_string()),
            }
            identity_view_from_engine(&engine, &identity.name)?
        };

        match should_wake_identity(&identity_view, &wake_settings, today) {
            WakeDecision::Skip(reason) => {
                if !should_log_wake_skip(&reason) {
                    continue;
                }
                emit_and_append_log(
                    app,
                    wake_audit_entry(
                        &run_id,
                        &identity_view,
                        "info",
                        "skipped",
                        Some(reason),
                        "账号未唤醒",
                        serde_json::json!({ "trigger": mode.as_str() }),
                        &wake_settings,
                    ),
                );
                continue;
            }
            WakeDecision::Wake => {}
        }

        let before_primary = identity_view.quota.primary_percent;
        let before_primary_reset_at = identity_view.quota.primary_reset_at;
        let result = run_wake_prompt(
            &settings.codex_binary,
            &identity,
            &wake_settings.message,
            Duration::from_secs(WAKE_TIMEOUT_SECONDS),
        );
        let after_refresh = read_quota_snapshot(&settings, &identity);
        let after_view = {
            let mut engine = state
                .engine
                .lock()
                .map_err(|_| "Modex state lock poisoned".to_string())?;
            match after_refresh {
                Ok(snapshot) => engine.set_snapshot(&identity.name, snapshot),
                Err(error) => engine.set_error(&identity.name, error.to_string()),
            }
            identity_view_from_engine(&engine, &identity.name)?
        };
        let after_primary = after_view.quota.primary_percent;
        let after_primary_reset_at = after_view.quota.primary_reset_at;
        let quota_observed_at_secs = (timestamp_millis() / 1000) as i64;
        let quota_evidence = wake_quota_evidence(
            before_primary,
            before_primary_reset_at,
            after_primary,
            after_primary_reset_at,
            quota_observed_at_secs,
        );
        let mut detail = serde_json::json!({
            "trigger": mode.as_str(),
            "beforePrimaryPercent": before_primary,
            "afterPrimaryPercent": after_primary,
            "beforePrimaryResetAt": before_primary_reset_at,
            "afterPrimaryResetAt": after_primary_reset_at,
            "quotaObservedAt": quota_observed_at_secs,
            "quotaWindowVerified": quota_evidence.is_verified(),
            "quotaWindowEvidence": quota_evidence.code()
        });
        let mut audit_view = after_view.clone();
        let (level, decision, title, reason) = match result {
            Ok(result) => {
                detail["wakeResult"] =
                    serde_json::to_value(&result).unwrap_or_else(|_| serde_json::Value::Null);
                if result.timed_out {
                    circuit_breaker = true;
                    (
                        "error",
                        "circuitBreaker",
                        "每日唤醒熔断",
                        Some("timedOut".to_string()),
                    )
                } else if result.exit_code != Some(0) {
                    circuit_breaker = true;
                    (
                        "error",
                        "circuitBreaker",
                        "每日唤醒熔断",
                        Some("nonZeroExit".to_string()),
                    )
                } else if result.last_message.trim() != "OK" || result.last_message.len() > 16 {
                    circuit_breaker = true;
                    (
                        "error",
                        "circuitBreaker",
                        "每日唤醒熔断",
                        Some("unexpectedReply".to_string()),
                    )
                } else if primary_delta_exceeds_limit(
                    before_primary,
                    after_primary,
                    wake_settings.max_primary_delta_percent,
                ) {
                    circuit_breaker = true;
                    (
                        "error",
                        "circuitBreaker",
                        "每日唤醒熔断",
                        Some("primaryDeltaExceeded".to_string()),
                    )
                } else if !quota_evidence.is_verified() {
                    detail["quotaSettleDelaySeconds"] =
                        serde_json::json!(WAKE_QUOTA_SETTLE_SECONDS);
                    std::thread::sleep(Duration::from_secs(WAKE_QUOTA_SETTLE_SECONDS));
                    let settled_refresh = read_quota_snapshot(&settings, &identity);
                    let settled_view = {
                        let mut engine = state
                            .engine
                            .lock()
                            .map_err(|_| "Modex state lock poisoned".to_string())?;
                        match settled_refresh {
                            Ok(snapshot) => engine.set_snapshot(&identity.name, snapshot),
                            Err(error) => engine.set_error(&identity.name, error.to_string()),
                        }
                        identity_view_from_engine(&engine, &identity.name)?
                    };
                    let settled_primary = settled_view.quota.primary_percent;
                    let settled_primary_reset_at = settled_view.quota.primary_reset_at;
                    let settled_observed_at_secs = (timestamp_millis() / 1000) as i64;
                    let settled_evidence = wake_quota_evidence(
                        before_primary,
                        before_primary_reset_at,
                        settled_primary,
                        settled_primary_reset_at,
                        settled_observed_at_secs,
                    );
                    detail["settledPrimaryPercent"] = serde_json::json!(settled_primary);
                    detail["settledPrimaryResetAt"] = serde_json::json!(settled_primary_reset_at);
                    detail["settledQuotaObservedAt"] = serde_json::json!(settled_observed_at_secs);
                    detail["settledQuotaWindowVerified"] =
                        serde_json::json!(settled_evidence.is_verified());
                    detail["settledQuotaWindowEvidence"] =
                        serde_json::json!(settled_evidence.code());
                    audit_view = settled_view;
                    let final_evidence = finalize_wake_quota_evidence(
                        quota_evidence.clone(),
                        Some(settled_evidence),
                    );
                    detail["quotaWindowVerified"] = serde_json::json!(final_evidence.is_verified());
                    detail["quotaWindowEvidence"] = serde_json::json!(final_evidence.code());
                    if final_evidence.is_verified() {
                        ("info", "woke", "每日唤醒完成", None)
                    } else {
                        (
                            "warn",
                            "unverified",
                            "每日唤醒待确认",
                            Some("quotaWindowUnverified".to_string()),
                        )
                    }
                } else {
                    ("info", "woke", "每日唤醒完成", None)
                }
            }
            Err(error) => {
                circuit_breaker = true;
                detail["error"] = serde_json::json!(error.to_string());
                (
                    "error",
                    "circuitBreaker",
                    "每日唤醒熔断",
                    Some("executionError".to_string()),
                )
            }
        };
        let mut entry = wake_audit_entry(
            &run_id,
            &audit_view,
            level,
            decision,
            None,
            title,
            detail,
            &wake_settings,
        );
        entry.reason = reason;
        emit_and_append_log(app, entry);
    }
    drop(guard);
    refresh_tray(app);
    emit_refresh_finished(app);
    emit_state_updated(app);
    Ok(())
}

fn wake_audit_entry(
    run_id: &str,
    identity: &IdentityView,
    level: &str,
    decision: &str,
    reason: Option<WakeSkipReason>,
    title: &str,
    detail: serde_json::Value,
    settings: &DailyWakeSettings,
) -> WakeAuditEntry {
    let reason = reason.map(|reason| format!("{reason:?}"));
    WakeAuditEntry {
        id: format!("{run_id}:{}:{}", identity.name, timestamp_millis()),
        run_id: run_id.to_string(),
        timestamp_millis: timestamp_millis(),
        level: level.to_string(),
        source: "dailyWake".to_string(),
        identity_name: Some(identity.name.clone()),
        title: title.to_string(),
        message: match decision {
            "skipped" => format!(
                "{} 未唤醒：{}",
                identity.name,
                reason.as_deref().unwrap_or("unknown")
            ),
            "woke" => format!("{} 已完成每日唤醒", identity.name),
            "unverified" => format!("{} 命令成功，但额度窗口未确认", identity.name),
            _ => format!("{} 每日唤醒已停止", identity.name),
        },
        decision: decision.to_string(),
        reason,
        primary_used_percent: Some(identity.quota.primary_percent),
        weekly_remaining_percent: Some(weekly_remaining_percent(identity)),
        thresholds: WakeThresholds::from(settings),
        detail,
    }
}

fn emit_and_append_log(app: &AppHandle, entry: WakeAuditEntry) {
    if let Err(error) = append_wake_log_entry(&default_wake_log_path(), &entry) {
        eprintln!("modex wake audit log failed: {error}");
    }
    let _ = app.emit(LOG_ENTRY_EVENT, entry);
}

fn local_date_and_time() -> Option<(String, String)> {
    #[cfg(target_os = "windows")]
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "Get-Date -Format 'yyyy-MM-dd:HH:mm'",
        ])
        .output()
        .ok()?;

    #[cfg(not(target_os = "windows"))]
    let output = Command::new("date").arg("+%Y-%m-%d:%H:%M").output().ok()?;

    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let (date, time) = raw.rsplit_once(':')?;
    let (date, hour) = date.rsplit_once(':')?;
    Some((date.to_string(), format!("{hour}:{time}")))
}

fn minutes_since_midnight(value: &str) -> Option<u16> {
    let (hour, minute) = value.split_once(':')?;
    let hour = hour.parse::<u16>().ok()?;
    let minute = minute.parse::<u16>().ok()?;
    if hour > 23 || minute > 59 {
        return None;
    }
    Some(hour * 60 + minute)
}

fn daily_wake_times(settings: &DailyWakeSettings) -> Vec<&str> {
    if settings.times.is_empty() {
        vec![settings.time.as_str()]
    } else {
        settings.times.iter().map(String::as_str).collect()
    }
}

fn wake_slot_key(today: &str, time: &str) -> String {
    format!("{today}#{time}")
}

fn is_known_non_team_identity(identity: &AppIdentity) -> bool {
    matches!(
        known_identity_plan_label(identity).as_deref(),
        Some(plan) if plan != "团队版"
    )
}

fn known_identity_plan_label(identity: &AppIdentity) -> Option<String> {
    let auth_label = plan_label(auth_plan_type(&identity.codex_home).as_deref());
    if auth_label != "计划未知" {
        return Some(auth_label);
    }
    let (_, suffix) = identity.name.rsplit_once(" · ")?;
    matches!(suffix, "团队版" | "企业版" | "个人版" | "免费版").then(|| suffix.to_string())
}

fn should_log_wake_skip(reason: &WakeSkipReason) -> bool {
    !matches!(reason, WakeSkipReason::NotTeamPlan)
}

fn refresh_tray(app: &AppHandle) {
    let _ = crate::tray::refresh_menu(app);
}

fn send_refresh_notifications(app: &AppHandle, before: &[IdentityView], after: &[IdentityView]) {
    send_notifications(app, &refresh_notifications(before, after));
}

fn current_identity_views(state: &ModexState) -> Result<Vec<IdentityView>, String> {
    state
        .engine
        .lock()
        .map_err(|_| "Modex state lock poisoned".to_string())
        .map(|engine| engine.app_state().identities)
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
    with_engine_ref(state.inner(), action)
}

fn with_engine_ref<T>(
    state: &ModexState,
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

async fn run_blocking<T: Send + 'static>(
    action: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(action)
        .await
        .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::time::Duration;

    use crate::core::app_config::{AppIdentity, AppSettings, DailyWakeSettings};
    use crate::core::quota::QuotaSnapshot;
    use crate::core::ModexError;

    use super::*;

    #[test]
    #[cfg(target_os = "macos")]
    fn showing_main_window_preserves_menu_bar_activation_policy() {
        assert!(!show_main_window_uses_regular_activation_policy());
    }

    fn jwt_with_claims(claims: serde_json::Value) -> String {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use base64::Engine;
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"none"}"#);
        let payload = URL_SAFE_NO_PAD.encode(claims.to_string());
        format!("{header}.{payload}.signature")
    }

    fn auth_json(email: &str, sub: &str, account_id: &str, plan_type: &str) -> String {
        let token = jwt_with_claims(serde_json::json!({
            "email": email,
            "sub": sub,
            "https://api.openai.com/auth": {
                "account_id": account_id,
                "chatgpt_plan_type": plan_type
            }
        }));
        serde_json::json!({
            "tokens": {
                "account_id": account_id,
                "id_token": token
            }
        })
        .to_string()
    }

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
    fn refresh_all_with_reader_updates_current_identity_from_source_auth() {
        let temp = assert_fs::TempDir::new().unwrap();
        let source_home = temp.path().join("source");
        let team_home = temp.path().join(".modex/111111111111");
        let backup_home = temp.path().join(".modex/222222222222");
        std::fs::create_dir_all(&source_home).unwrap();
        std::fs::create_dir_all(&team_home).unwrap();
        std::fs::create_dir_all(&backup_home).unwrap();
        std::fs::write(
            team_home.join("auth.json"),
            auth_json("team@example.com", "user-team", "acct-team", "team"),
        )
        .unwrap();
        let backup_auth = auth_json("backup@example.com", "user-backup", "acct-backup", "team");
        std::fs::write(backup_home.join("auth.json"), &backup_auth).unwrap();
        std::fs::write(source_home.join("auth.json"), backup_auth).unwrap();
        let mut settings = AppSettings::default_for_home(temp.path().to_path_buf());
        settings.source_home = source_home;
        settings.current_identity_name = Some("team@example.com · 团队版".to_string());
        settings.identities.push(AppIdentity {
            name: "team@example.com · 团队版".to_string(),
            codex_home: team_home,
            monitor: false,
            workspace_id: None,
        });
        settings.identities.push(AppIdentity {
            name: "backup@example.com · 团队版".to_string(),
            codex_home: backup_home,
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

        let team = refreshed
            .iter()
            .find(|identity| identity.name == "team@example.com · 团队版")
            .unwrap();
        let backup = refreshed
            .iter()
            .find(|identity| identity.name == "backup@example.com · 团队版")
            .unwrap();
        assert!(!team.is_current);
        assert!(backup.is_current);
    }

    #[test]
    fn refresh_all_with_reader_marks_missing_auth_as_login_expired() {
        let temp = assert_fs::TempDir::new().unwrap();
        let mut settings = AppSettings::default_for_home(temp.path().to_path_buf());
        settings.identities.push(AppIdentity {
            name: "Expired".to_string(),
            codex_home: temp.path().join(".modex/expired"),
            monitor: false,
            workspace_id: None,
        });
        let state = ModexState::new(AppEngine::new(settings, temp.path().join("config.json")));
        let guard = state.try_begin_refresh().unwrap();

        let refreshed = refresh_all_with_reader(&state, guard, |_settings, _identity| {
            Err(ModexError::from(
                "账号缺少登录凭据：/tmp/.modex/expired/auth.json",
            ))
        })
        .unwrap();

        assert!(refreshed[0].login_expired);
        assert!(!refreshed[0].logged_in);
        assert_eq!(refreshed[0].quota.status, "error");
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

    #[test]
    fn daily_wake_starts_only_inside_the_scheduled_window_once_per_slot() {
        let mut settings = DailyWakeSettings {
            enabled: true,
            time: "08:30".to_string(),
            times: vec!["08:30".to_string()],
            ..DailyWakeSettings::default()
        };

        assert!(!should_start_daily_wake(&settings, "2026-05-18", "08:29"));
        assert!(should_start_daily_wake(&settings, "2026-05-18", "08:30"));
        assert!(should_start_daily_wake(&settings, "2026-05-18", "08:35"));
        assert!(!should_start_daily_wake(&settings, "2026-05-18", "08:36"));

        settings.last_run_slots = vec!["2026-05-18#08:30".to_string()];
        assert!(!should_start_daily_wake(&settings, "2026-05-18", "08:30"));
        assert!(should_start_daily_wake(&settings, "2026-05-19", "08:30"));
    }

    #[test]
    fn daily_wake_can_start_multiple_slots_on_the_same_day() {
        let mut settings = DailyWakeSettings {
            enabled: true,
            time: "08:30".to_string(),
            times: vec!["08:30".to_string(), "14:00".to_string()],
            ..DailyWakeSettings::default()
        };

        assert_eq!(
            pending_daily_wake_time(&settings, "2026-05-18", "08:32").as_deref(),
            Some("08:30")
        );

        settings.last_run_slots = vec!["2026-05-18#08:30".to_string()];
        assert_eq!(
            pending_daily_wake_time(&settings, "2026-05-18", "14:02").as_deref(),
            Some("14:00")
        );
        assert!(!should_start_daily_wake(&settings, "2026-05-18", "08:33"));
    }

    #[test]
    fn daily_wake_ignores_legacy_last_run_date_without_slot_marker() {
        let mut settings = DailyWakeSettings {
            enabled: true,
            time: "08:30".to_string(),
            times: vec!["08:30".to_string()],
            ..DailyWakeSettings::default()
        };
        settings.last_run_date = Some("2026-05-18".to_string());
        settings.last_run_slots = Vec::new();

        assert_eq!(
            pending_daily_wake_time(&settings, "2026-05-18", "08:30").as_deref(),
            Some("08:30")
        );
    }

    #[test]
    fn daily_wake_does_not_start_when_disabled_or_time_is_invalid() {
        let disabled = DailyWakeSettings {
            enabled: false,
            time: "08:30".to_string(),
            times: vec!["08:30".to_string()],
            ..DailyWakeSettings::default()
        };
        let invalid = DailyWakeSettings {
            enabled: true,
            time: "25:00".to_string(),
            times: vec!["25:00".to_string()],
            ..DailyWakeSettings::default()
        };

        assert!(!should_start_daily_wake(&disabled, "2026-05-18", "08:30"));
        assert!(!should_start_daily_wake(&invalid, "2026-05-18", "08:30"));
    }

    #[test]
    fn wake_logs_skip_reasons_only_for_team_candidates() {
        assert!(!should_log_wake_skip(&WakeSkipReason::NotTeamPlan));
        assert!(should_log_wake_skip(
            &WakeSkipReason::PrimaryUsageAboveThreshold
        ));
        assert!(should_log_wake_skip(&WakeSkipReason::QuotaUnavailable));
    }

    #[test]
    fn wake_excludes_known_non_team_identities_before_quota_check() {
        let free = AppIdentity {
            name: "free@example.com · 免费版".to_string(),
            codex_home: PathBuf::from("/tmp/free"),
            monitor: false,
            workspace_id: None,
        };
        let enterprise = AppIdentity {
            name: "enterprise@example.com · 企业版".to_string(),
            codex_home: PathBuf::from("/tmp/enterprise"),
            monitor: false,
            workspace_id: None,
        };
        let team = AppIdentity {
            name: "team@example.com · 团队版".to_string(),
            codex_home: PathBuf::from("/tmp/team"),
            monitor: false,
            workspace_id: None,
        };
        let unknown = AppIdentity {
            name: "unknown@example.com".to_string(),
            codex_home: PathBuf::from("/tmp/unknown"),
            monitor: false,
            workspace_id: None,
        };

        assert!(is_known_non_team_identity(&free));
        assert!(is_known_non_team_identity(&enterprise));
        assert!(!is_known_non_team_identity(&team));
        assert!(!is_known_non_team_identity(&unknown));
    }
}

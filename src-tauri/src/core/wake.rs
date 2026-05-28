use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::app_config::{AppIdentity, DailyWakeSettings};
use super::codex::{build_codex_env, resolve_codex_binary};
use super::engine::IdentityView;
use super::ModexResult;

const PRIMARY_WINDOW_SECONDS: i64 = 5 * 60 * 60;
const PRIMARY_WINDOW_ADVANCE_TOLERANCE_SECONDS: i64 = 10 * 60;
const WAKE_THREAD_START_REQUEST_ID: u8 = 2;
const WAKE_TURN_START_REQUEST_ID: u8 = 3;
const WAKE_THREAD_ARCHIVE_REQUEST_ID: u8 = 4;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WakeDecision {
    Wake,
    Skip(WakeSkipReason),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WakeSkipReason {
    AlreadyRanToday,
    LoginUnavailable,
    NotTeamPlan,
    QuotaUnavailable,
    PrimaryUsageAboveThreshold,
    WeeklyRemainingBelowThreshold,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WakeThresholds {
    pub skip_if_primary_used_above_percent: u8,
    pub skip_if_weekly_remaining_below_percent: u8,
    pub max_primary_delta_percent: u8,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WakeAuditEntry {
    pub id: String,
    pub run_id: String,
    pub timestamp_millis: u128,
    pub level: String,
    pub source: String,
    pub identity_name: Option<String>,
    pub title: String,
    pub message: String,
    pub decision: String,
    pub reason: Option<String>,
    pub primary_used_percent: Option<u8>,
    pub weekly_remaining_percent: Option<u8>,
    pub thresholds: WakeThresholds,
    pub detail: Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WakePromptResult {
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub last_message: String,
    pub stdout: String,
    pub stderr: String,
    pub thread_id: Option<String>,
    pub archived: bool,
    pub archive_error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct WakeArchiveResult {
    archived: bool,
    error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WakeQuotaEvidence {
    Verified(&'static str),
    Unverified(&'static str),
}

impl WakeQuotaEvidence {
    pub fn is_verified(&self) -> bool {
        matches!(self, Self::Verified(_))
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::Verified(code) | Self::Unverified(code) => code,
        }
    }
}

impl From<&DailyWakeSettings> for WakeThresholds {
    fn from(settings: &DailyWakeSettings) -> Self {
        Self {
            skip_if_primary_used_above_percent: settings.skip_if_primary_used_above_percent,
            skip_if_weekly_remaining_below_percent: settings.skip_if_weekly_remaining_below_percent,
            max_primary_delta_percent: settings.max_primary_delta_percent,
        }
    }
}

pub fn should_wake_identity(
    identity: &IdentityView,
    settings: &DailyWakeSettings,
    today: &str,
) -> WakeDecision {
    if settings.last_run_date.as_deref() == Some(today) {
        return WakeDecision::Skip(WakeSkipReason::AlreadyRanToday);
    }
    if !identity.logged_in || identity.login_expired {
        return WakeDecision::Skip(WakeSkipReason::LoginUnavailable);
    }
    if identity.quota.plan != "团队版" {
        return WakeDecision::Skip(WakeSkipReason::NotTeamPlan);
    }
    if matches!(identity.quota.status.as_str(), "unknown" | "error")
        || identity.quota.primary_label.is_empty()
        || identity.quota.secondary_label.is_empty()
    {
        return WakeDecision::Skip(WakeSkipReason::QuotaUnavailable);
    }
    if identity.quota.primary_percent > settings.skip_if_primary_used_above_percent {
        return WakeDecision::Skip(WakeSkipReason::PrimaryUsageAboveThreshold);
    }
    if weekly_remaining_percent(identity) < settings.skip_if_weekly_remaining_below_percent {
        return WakeDecision::Skip(WakeSkipReason::WeeklyRemainingBelowThreshold);
    }
    if identity.quota.status != "available" {
        return WakeDecision::Skip(WakeSkipReason::QuotaUnavailable);
    }
    WakeDecision::Wake
}

pub fn weekly_remaining_percent(identity: &IdentityView) -> u8 {
    100u8.saturating_sub(identity.quota.secondary_percent.min(100))
}

pub fn primary_delta_exceeds_limit(before: u8, after: u8, max_delta: u8) -> bool {
    if after < before {
        return false;
    }
    after.saturating_sub(before) > max_delta
}

pub fn wake_quota_evidence(
    before_primary_percent: u8,
    before_primary_reset_at: Option<i64>,
    after_primary_percent: u8,
    after_primary_reset_at: Option<i64>,
    observed_at_secs: i64,
) -> WakeQuotaEvidence {
    let Some(after_reset_at) = after_primary_reset_at else {
        return WakeQuotaEvidence::Unverified("missingPrimaryResetAt");
    };
    if after_reset_at <= observed_at_secs {
        return WakeQuotaEvidence::Unverified("expiredPrimaryResetAt");
    }
    if after_primary_percent > before_primary_percent {
        return WakeQuotaEvidence::Verified("primaryUsageIncreased");
    }
    match before_primary_reset_at {
        None => WakeQuotaEvidence::Verified("primaryWindowAppeared"),
        Some(before_reset_at) if before_reset_at == after_reset_at => {
            WakeQuotaEvidence::Verified("primaryWindowStable")
        }
        Some(before_reset_at)
            if before_reset_at < after_reset_at
                && looks_like_fresh_primary_window(after_reset_at, observed_at_secs) =>
        {
            WakeQuotaEvidence::Verified("primaryWindowAdvanced")
        }
        Some(before_reset_at) if before_reset_at < after_reset_at => {
            WakeQuotaEvidence::Unverified("primaryWindowMovedWithoutUsage")
        }
        Some(_) => WakeQuotaEvidence::Unverified("primaryWindowMovedBackward"),
    }
}

fn looks_like_fresh_primary_window(reset_at: i64, observed_at_secs: i64) -> bool {
    let seconds_until_reset = reset_at.saturating_sub(observed_at_secs);
    let minimum = PRIMARY_WINDOW_SECONDS - PRIMARY_WINDOW_ADVANCE_TOLERANCE_SECONDS;
    let maximum = PRIMARY_WINDOW_SECONDS + PRIMARY_WINDOW_ADVANCE_TOLERANCE_SECONDS;
    (minimum..=maximum).contains(&seconds_until_reset)
}

pub fn finalize_wake_quota_evidence(
    initial: WakeQuotaEvidence,
    settled: Option<WakeQuotaEvidence>,
) -> WakeQuotaEvidence {
    if initial.is_verified() {
        return initial;
    }
    settled.unwrap_or(initial)
}

pub fn append_wake_log_entry(path: &Path, entry: &WakeAuditEntry) -> ModexResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{}", serde_json::to_string(entry)?)?;
    Ok(())
}

pub fn read_recent_wake_log_entries(path: &Path, limit: usize) -> ModexResult<Vec<WakeAuditEntry>> {
    if !path.exists() || limit == 0 {
        return Ok(Vec::new());
    }
    let file = OpenOptions::new().read(true).open(path)?;
    let mut entries = Vec::new();
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<WakeAuditEntry>(trimmed) {
            entries.push(entry);
        }
    }
    entries.reverse();
    entries.truncate(limit);
    Ok(entries)
}

pub fn default_wake_log_path() -> PathBuf {
    dirs::data_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Modex")
        .join("wake-runs.jsonl")
}

pub fn timestamp_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

pub fn run_wake_prompt(
    codex_binary: &str,
    identity: &AppIdentity,
    message: &str,
    timeout: Duration,
) -> ModexResult<WakePromptResult> {
    let workdir = tempfile::Builder::new()
        .prefix("modex-wake-work-")
        .tempdir()?;
    let mut child = Command::new(resolve_codex_binary(codex_binary))
        .arg("app-server")
        .arg("--listen")
        .arg("stdio://")
        .envs(build_codex_env(&identity.codex_home))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| super::ModexError::from("app-server stdin is unavailable"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| super::ModexError::from("app-server stdout is unavailable"))?;
    let stdout_reader = read_json_lines_in_background(stdout);
    let stderr_reader = child.stderr.take().map(read_pipe_in_background);
    write_app_server_request(&mut stdin, 1, "initialize", initialize_params())?;
    write_app_server_request(
        &mut stdin,
        WAKE_THREAD_START_REQUEST_ID,
        "thread/start",
        wake_thread_start_params(workdir.path()),
    )?;

    let started = Instant::now();
    let mut stdout_lines = Vec::new();
    let mut thread_id: Option<String> = None;
    let mut turn_id: Option<String> = None;
    let mut turn_started = false;
    let mut completed = false;
    let mut failed = false;
    let mut timed_out = false;
    let mut last_message = String::new();
    let mut failure_message = String::new();
    let mut archive_result = WakeArchiveResult::default();

    while !completed && !failed {
        if started.elapsed() >= timeout {
            timed_out = true;
            break;
        }
        if let Some(status) = child.try_wait()? {
            failure_message = format!("app-server exited before turn completed: {status}");
            break;
        }
        let remaining = timeout
            .checked_sub(started.elapsed())
            .unwrap_or_else(|| Duration::from_millis(0));
        let wait = remaining.min(Duration::from_millis(100));
        match stdout_reader.recv_timeout(wait) {
            Ok((line, server_message)) => {
                stdout_lines.push(line);
                if is_response_error(&server_message, WAKE_THREAD_START_REQUEST_ID) {
                    failed = true;
                    failure_message =
                        app_server_error_message(&server_message, "thread/start failed");
                } else if is_response_error(&server_message, WAKE_TURN_START_REQUEST_ID) {
                    failed = true;
                    failure_message =
                        app_server_error_message(&server_message, "turn/start failed");
                } else if !turn_started
                    && server_message.get("id").and_then(Value::as_u64)
                        == Some(WAKE_THREAD_START_REQUEST_ID as u64)
                {
                    let Some(next_thread_id) = app_server_thread_id(&server_message) else {
                        failed = true;
                        failure_message = "thread/start returned no thread id".to_string();
                        continue;
                    };
                    write_app_server_request(
                        &mut stdin,
                        WAKE_TURN_START_REQUEST_ID,
                        "turn/start",
                        wake_turn_start_params(&next_thread_id, message),
                    )?;
                    thread_id = Some(next_thread_id);
                    turn_started = true;
                } else if server_message.get("id").and_then(Value::as_u64)
                    == Some(WAKE_TURN_START_REQUEST_ID as u64)
                {
                    turn_id = app_server_turn_id(&server_message);
                } else if is_agent_message_completed(
                    &server_message,
                    thread_id.as_deref(),
                    turn_id.as_deref(),
                ) {
                    if let Some(text) = agent_message_text(&server_message) {
                        last_message = text;
                    }
                } else if is_turn_completed(
                    &server_message,
                    thread_id.as_deref(),
                    turn_id.as_deref(),
                ) {
                    if let Some(error) = turn_error_message(&server_message) {
                        failed = true;
                        failure_message = error;
                    } else {
                        completed = true;
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                failed = true;
                failure_message = "app-server stdout closed before turn completed".to_string();
            }
        }
    }
    if let Some(thread_id) = thread_id.as_deref() {
        archive_result = match archive_wake_thread(
            &mut stdin,
            &stdout_reader,
            &mut stdout_lines,
            thread_id,
            Duration::from_secs(2),
        ) {
            Ok(result) => result,
            Err(error) => WakeArchiveResult {
                archived: false,
                error: Some(error.to_string()),
            },
        };
    }
    if child.try_wait()?.is_none() {
        let _ = child.kill();
    }
    let _ = child.wait();
    let stderr = join_pipe_reader(stderr_reader)?;
    if !failure_message.is_empty() && last_message.is_empty() {
        last_message = failure_message.clone();
    }

    Ok(WakePromptResult {
        exit_code: if completed { Some(0) } else { Some(1) },
        timed_out,
        last_message,
        stdout: stdout_lines.join("\n"),
        stderr,
        thread_id,
        archived: archive_result.archived,
        archive_error: archive_result.error,
    })
}

fn read_pipe_in_background(
    mut pipe: impl Read + Send + 'static,
) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut value = String::new();
        let _ = pipe.read_to_string(&mut value);
        value
    })
}

fn join_pipe_reader(reader: Option<std::thread::JoinHandle<String>>) -> ModexResult<String> {
    let Some(reader) = reader else {
        return Ok(String::new());
    };
    Ok(reader.join().unwrap_or_default())
}

fn wake_prompt(message: &str) -> String {
    format!(
        "Modex daily wake check. Reply exactly OK. User message: {}",
        message.trim()
    )
}

fn wake_thread_start_params(workdir: &Path) -> Value {
    serde_json::json!({
        "cwd": workdir.display().to_string(),
        "approvalPolicy": "never",
        "sandbox": "read-only",
        "ephemeral": false,
        "threadSource": "user",
        "sessionStartSource": "startup",
        "environments": [],
        "dynamicTools": [],
        "config": {
            "web_search": "disabled",
        },
        "baseInstructions": "You are running a Modex daily wake check.",
        "developerInstructions": "Reply exactly OK. Do not inspect files, run commands, use tools, analyze the project, or add explanation.",
        "experimentalRawEvents": false,
        "persistExtendedHistory": false,
    })
}

fn wake_turn_start_params(thread_id: &str, message: &str) -> Value {
    serde_json::json!({
        "threadId": thread_id,
        "input": [{
            "type": "text",
            "text": wake_prompt(message),
            "text_elements": [],
        }],
        "approvalPolicy": "never",
        "sandboxPolicy": {
            "type": "readOnly",
            "networkAccess": false,
        },
        "effort": "low",
    })
}

fn initialize_params() -> Value {
    serde_json::json!({
        "clientInfo": {
            "name": "modex",
            "title": "Modex",
            "version": "0.1.0"
        },
        "capabilities": {
            "experimentalApi": true,
            "optOutNotificationMethods": []
        }
    })
}

fn write_app_server_request(
    stdin: &mut impl Write,
    id: u8,
    method: &str,
    params: Value,
) -> ModexResult<()> {
    let request = serde_json::json!({
        "id": id,
        "method": method,
        "params": params
    });
    writeln!(stdin, "{request}")?;
    stdin.flush()?;
    Ok(())
}

fn read_json_lines_in_background(pipe: impl Read + Send + 'static) -> Receiver<(String, Value)> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(pipe).lines().map_while(Result::ok) {
            if let Ok(message) = serde_json::from_str::<Value>(&line) {
                let _ = tx.send((line, message));
            }
        }
    });
    rx
}

fn app_server_thread_id(message: &Value) -> Option<String> {
    message
        .get("result")?
        .get("thread")?
        .get("id")?
        .as_str()
        .map(ToString::to_string)
}

fn app_server_turn_id(message: &Value) -> Option<String> {
    message
        .get("result")?
        .get("turn")?
        .get("id")?
        .as_str()
        .map(ToString::to_string)
}

fn is_response_error(message: &Value, id: u8) -> bool {
    message.get("id").and_then(Value::as_u64) == Some(id as u64) && message.get("error").is_some()
}

fn app_server_error_message(message: &Value, fallback: &str) -> String {
    message
        .get("error")
        .map(Value::to_string)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

fn is_agent_message_completed(
    message: &Value,
    expected_thread_id: Option<&str>,
    expected_turn_id: Option<&str>,
) -> bool {
    if message.get("method").and_then(Value::as_str) != Some("item/completed") {
        return false;
    }
    if !matches_expected_id(message, "threadId", expected_thread_id) {
        return false;
    }
    if !matches_expected_id(message, "turnId", expected_turn_id) {
        return false;
    }
    message
        .get("params")
        .and_then(|params| params.get("item"))
        .and_then(|item| item.get("type"))
        .and_then(Value::as_str)
        == Some("agentMessage")
}

fn agent_message_text(message: &Value) -> Option<String> {
    message
        .get("params")?
        .get("item")?
        .get("text")?
        .as_str()
        .map(ToString::to_string)
}

fn is_turn_completed(
    message: &Value,
    expected_thread_id: Option<&str>,
    expected_turn_id: Option<&str>,
) -> bool {
    if message.get("method").and_then(Value::as_str) != Some("turn/completed") {
        return false;
    }
    if !matches_expected_id(message, "threadId", expected_thread_id) {
        return false;
    }
    let Some(expected_turn_id) = expected_turn_id else {
        return true;
    };
    message
        .get("params")
        .and_then(|params| params.get("turn"))
        .and_then(|turn| turn.get("id"))
        .and_then(Value::as_str)
        == Some(expected_turn_id)
}

fn turn_error_message(message: &Value) -> Option<String> {
    let turn = message.get("params")?.get("turn")?;
    if turn.get("status").and_then(Value::as_str) != Some("failed") {
        return None;
    }
    let error = turn.get("error")?;
    error
        .get("message")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .or_else(|| Some(error.to_string()))
}

fn matches_expected_id(message: &Value, field: &str, expected: Option<&str>) -> bool {
    let Some(expected) = expected else {
        return true;
    };
    message
        .get("params")
        .and_then(|params| params.get(field))
        .and_then(Value::as_str)
        == Some(expected)
}

fn archive_wake_thread(
    stdin: &mut impl Write,
    stdout_reader: &Receiver<(String, Value)>,
    stdout_lines: &mut Vec<String>,
    thread_id: &str,
    timeout: Duration,
) -> ModexResult<WakeArchiveResult> {
    write_app_server_request(
        stdin,
        WAKE_THREAD_ARCHIVE_REQUEST_ID,
        "thread/archive",
        serde_json::json!({ "threadId": thread_id }),
    )?;
    let started = Instant::now();
    while started.elapsed() < timeout {
        match stdout_reader.recv_timeout(Duration::from_millis(100)) {
            Ok((line, message)) => {
                stdout_lines.push(line);
                if let Some(result) =
                    archive_result_from_message(&message, WAKE_THREAD_ARCHIVE_REQUEST_ID, thread_id)
                {
                    return Ok(result);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Ok(WakeArchiveResult {
                    archived: false,
                    error: Some("app-server stdout closed before thread/archive".to_string()),
                });
            }
        }
    }
    Ok(WakeArchiveResult {
        archived: false,
        error: Some("timed out waiting for thread/archive".to_string()),
    })
}

fn archive_result_from_message(
    message: &Value,
    archive_request_id: u8,
    thread_id: &str,
) -> Option<WakeArchiveResult> {
    if message.get("id").and_then(Value::as_u64) == Some(archive_request_id as u64) {
        if message.get("result").is_some() {
            return Some(WakeArchiveResult {
                archived: true,
                error: None,
            });
        }
        let error = message
            .get("error")
            .map(app_server_error_text)
            .or_else(|| Some("thread/archive failed".to_string()));
        return Some(WakeArchiveResult {
            archived: false,
            error,
        });
    }
    if message.get("method").and_then(Value::as_str) == Some("thread/archived")
        && message
            .get("params")
            .and_then(|params| params.get("threadId"))
            .and_then(Value::as_str)
            == Some(thread_id)
    {
        return Some(WakeArchiveResult {
            archived: true,
            error: None,
        });
    }
    None
}

fn app_server_error_text(error: &Value) -> String {
    error
        .get("message")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .unwrap_or_else(|| error.to_string())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{archive_result_from_message, wake_thread_start_params, wake_turn_start_params};

    #[test]
    fn wake_uses_normal_app_server_thread_turn() {
        let thread = wake_thread_start_params(Path::new("/tmp/wake-work"));
        assert_eq!(thread["cwd"], "/tmp/wake-work");
        assert_eq!(thread["ephemeral"], false);
        assert_eq!(thread["approvalPolicy"], "never");
        assert_eq!(thread["sandbox"], "read-only");
        assert_eq!(thread["threadSource"], "user");
        assert_eq!(thread["environments"].as_array().unwrap().len(), 0);
        assert_eq!(thread["dynamicTools"].as_array().unwrap().len(), 0);
        assert_eq!(thread["config"]["web_search"], "disabled");

        let turn = wake_turn_start_params("thread-1", "Good morning");
        assert_eq!(turn["threadId"], "thread-1");
        assert_eq!(turn["approvalPolicy"], "never");
        assert_eq!(turn["sandboxPolicy"]["type"], "readOnly");
        assert_eq!(turn["sandboxPolicy"]["networkAccess"], false);
        assert_eq!(turn["effort"], "low");
        assert_eq!(
            turn["input"][0]["text"],
            "Modex daily wake check. Reply exactly OK. User message: Good morning"
        );
        assert_eq!(
            turn["input"][0]["text_elements"].as_array().unwrap().len(),
            0
        );
    }

    #[test]
    fn wake_archive_result_records_success_and_errors() {
        let success = archive_result_from_message(
            &serde_json::json!({ "id": 4, "result": {} }),
            4,
            "thread-1",
        )
        .unwrap();
        assert!(success.archived);
        assert_eq!(success.error, None);

        let notification = archive_result_from_message(
            &serde_json::json!({
                "method": "thread/archived",
                "params": { "threadId": "thread-1" }
            }),
            4,
            "thread-1",
        )
        .unwrap();
        assert!(notification.archived);

        let failure = archive_result_from_message(
            &serde_json::json!({
                "id": 4,
                "error": { "message": "archive failed" }
            }),
            4,
            "thread-1",
        )
        .unwrap();
        assert!(!failure.archived);
        assert_eq!(failure.error.as_deref(), Some("archive failed"));
    }
}

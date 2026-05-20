use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::app_config::{AppIdentity, DailyWakeSettings};
use super::codex::{build_codex_env, resolve_codex_binary};
use super::engine::IdentityView;
use super::ModexResult;

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
    let last_message_path = workdir.path().join("last-message.txt");
    let prompt = wake_prompt(message);
    let mut child = Command::new(resolve_codex_binary(codex_binary))
        .args(wake_exec_args(workdir.path(), &last_message_path, prompt))
        .envs(build_codex_env(&identity.codex_home))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout_reader = child.stdout.take().map(read_pipe_in_background);
    let stderr_reader = child.stderr.take().map(read_pipe_in_background);

    let started = Instant::now();
    let mut timed_out = false;
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if started.elapsed() >= timeout {
            timed_out = true;
            let _ = child.kill();
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let status = child.wait()?;
    let stdout = join_pipe_reader(stdout_reader)?;
    let stderr = join_pipe_reader(stderr_reader)?;
    let last_message = std::fs::read_to_string(last_message_path).unwrap_or_default();

    Ok(WakePromptResult {
        exit_code: status.code(),
        timed_out,
        last_message,
        stdout,
        stderr,
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

fn wake_exec_args(workdir: &Path, last_message_path: &Path, prompt: String) -> Vec<OsString> {
    vec![
        "exec".into(),
        "--ephemeral".into(),
        "--ignore-user-config".into(),
        "--ignore-rules".into(),
        "--json".into(),
        "--skip-git-repo-check".into(),
        "-C".into(),
        workdir.as_os_str().to_os_string(),
        "-s".into(),
        "read-only".into(),
        "-o".into(),
        last_message_path.as_os_str().to_os_string(),
        prompt.into(),
    ]
}

fn wake_prompt(message: &str) -> String {
    format!(
        "Modex daily wake check. Reply exactly OK. Do not inspect files, run commands, use tools, analyze the project, or add explanation. User message: {}",
        message.trim()
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::wake_exec_args;

    #[test]
    fn wake_exec_args_match_current_codex_cli() {
        let args = wake_exec_args(
            Path::new("/tmp/wake-work"),
            Path::new("/tmp/wake-work/last-message.txt"),
            "Reply exactly OK".to_string(),
        )
        .into_iter()
        .map(|arg| arg.to_string_lossy().to_string())
        .collect::<Vec<_>>();

        assert!(args.contains(&"--ephemeral".to_string()));
        assert!(args.contains(&"--ignore-user-config".to_string()));
        assert!(args.contains(&"--ignore-rules".to_string()));
        assert!(args.contains(&"--json".to_string()));
        assert!(args.contains(&"--skip-git-repo-check".to_string()));
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "-s" && pair[1] == "read-only"));
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "-o" && pair[1] == "/tmp/wake-work/last-message.txt"));
        assert!(!args.contains(&"-a".to_string()));
        assert!(!args.contains(&"never".to_string()));
    }
}

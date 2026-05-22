use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::Value;
use tempfile::TempDir;

use super::app_config::{AppIdentity, AppSettings};
use super::auth::plan_label;
use super::quota::{snapshot_from_rate_limits, QuotaSnapshot};
use super::sync::{
    source_history_rollout_paths, sync_identity_auth, sync_source_history_provider,
    HistorySyncProvider,
};
use super::{ModexError, ModexResult};

const DEFAULT_CODEX_APP_CLI: &str = "/Applications/Codex.app/Contents/Resources/codex";
const MODEX_API_KEY_PROVIDER_ID: &str = "modex-api-key";
const MODEX_API_KEY_PROVIDER_NAME: &str = "Modex API Key";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramInvocation {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub envs: Vec<(String, String)>,
}

pub fn run_login(settings: &AppSettings, identity: &AppIdentity) -> ModexResult<()> {
    std::fs::create_dir_all(&identity.codex_home)?;
    let mut command = Command::new(resolve_codex_binary(&settings.codex_binary));
    command
        .arg("login")
        .arg("-c")
        .arg(r#"forced_login_method="chatgpt""#)
        .envs(build_codex_env(&identity.codex_home))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(workspace_id) = &identity.workspace_id {
        command.arg("-c").arg(format!(
            r#"forced_chatgpt_workspace_id="{}""#,
            escape_config_value(workspace_id)
        ));
    }
    command.spawn()?;
    Ok(())
}

pub fn api_key_login_invocation(
    settings: &AppSettings,
    identity: &AppIdentity,
) -> ProgramInvocation {
    ProgramInvocation {
        program: resolve_codex_binary(&settings.codex_binary),
        args: vec!["login".to_string(), "--with-api-key".to_string()],
        envs: build_codex_env(&identity.codex_home),
    }
}

pub fn run_api_key_login(
    settings: &AppSettings,
    identity: &AppIdentity,
    api_key: &str,
) -> ModexResult<()> {
    std::fs::create_dir_all(&identity.codex_home)?;
    let invocation = api_key_login_invocation(settings, identity);
    let mut child = Command::new(invocation.program)
        .args(invocation.args)
        .envs(invocation.envs)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| ModexError::from("codex login stdin is unavailable"))?;
    stdin.write_all(api_key.as_bytes())?;
    stdin.flush()?;
    drop(stdin);
    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(ModexError::from("API Key 登录失败"))
    }
}

pub fn open_codex_app(settings: &AppSettings, identity: &AppIdentity) -> ModexResult<()> {
    open_codex_app_with_operations(
        settings,
        identity,
        quit_codex_app_if_running,
        prepare_identity_for_launch,
        spawn_program,
    )
}

fn open_codex_app_with_operations(
    settings: &AppSettings,
    identity: &AppIdentity,
    quit: impl FnOnce(&AppSettings) -> ModexResult<()>,
    prepare: impl FnOnce(&AppSettings, &AppIdentity) -> ModexResult<()>,
    launch: impl FnOnce(ProgramInvocation) -> ModexResult<()>,
) -> ModexResult<()> {
    #[cfg(target_os = "macos")]
    quit(settings)?;
    #[cfg(not(target_os = "macos"))]
    let _ = quit;
    prepare(settings, identity)?;
    launch(open_codex_app_launch_command(settings))
}

pub fn prepare_identity_for_launch(
    settings: &AppSettings,
    identity: &AppIdentity,
) -> ModexResult<()> {
    prepare_identity_for_launch_with_operations(
        settings,
        identity,
        sync_identity_auth,
        apply_identity_runtime_config,
        sync_source_history_provider,
    )
}

pub fn apply_identity_runtime_config(
    settings: &AppSettings,
    identity: &AppIdentity,
) -> ModexResult<()> {
    apply_openai_base_url_config(&settings.source_home, identity.api_base_url.as_deref())
}

pub fn apply_openai_base_url_config(codex_home: &Path, base_url: Option<&str>) -> ModexResult<()> {
    fs::create_dir_all(codex_home)?;
    let config_path = codex_home.join("config.toml");
    let existing = fs::read_to_string(&config_path).unwrap_or_default();
    let base_url = base_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let mut lines = strip_managed_api_key_provider_config(&existing, base_url.is_some());
    if let Some(base_url) = base_url {
        let insert_at = top_level_insert_index(&lines);
        lines.insert(
            insert_at,
            format!("model_provider = \"{MODEX_API_KEY_PROVIDER_ID}\""),
        );
        if !lines.is_empty() && !lines.last().is_some_and(|line| line.trim().is_empty()) {
            lines.push(String::new());
        }
        lines.extend([
            format!("[model_providers.{MODEX_API_KEY_PROVIDER_ID}]"),
            format!("name = \"{MODEX_API_KEY_PROVIDER_NAME}\""),
            format!("base_url = \"{}\"", escape_config_value(&base_url)),
            "wire_api = \"responses\"".to_string(),
            "requires_openai_auth = true".to_string(),
            "supports_websockets = false".to_string(),
        ]);
    }
    let lines = tidy_config_lines(lines);
    let next = if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    };
    fs::write(config_path, next)?;
    Ok(())
}

fn prepare_identity_for_launch_with_operations(
    settings: &AppSettings,
    identity: &AppIdentity,
    sync_auth: impl FnOnce(&Path, &Path) -> ModexResult<PathBuf>,
    apply_config: impl FnOnce(&AppSettings, &AppIdentity) -> ModexResult<()>,
    sync_history: impl FnOnce(&Path, HistorySyncProvider) -> ModexResult<()>,
) -> ModexResult<()> {
    let backup = RuntimeHomeBackup::capture(&settings.source_home)?;
    let provider = HistorySyncProvider::from(&identity.auth_type);
    let result = (|| {
        sync_auth(&settings.source_home, &identity.codex_home)?;
        apply_config(settings, identity)?;
        sync_history(&settings.source_home, provider)
    })();
    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            if let Err(restore_error) = backup.restore() {
                return Err(ModexError::from(format!(
                    "{error}; 且运行时目录回滚失败：{restore_error}"
                )));
            }
            Err(error)
        }
    }
}

struct RuntimeHomeBackup {
    snapshots: Vec<FileSnapshot>,
    temp_dir: TempDir,
}

impl RuntimeHomeBackup {
    fn capture(source_home: &Path) -> ModexResult<Self> {
        let mut tracked_paths = vec![
            source_home.join("auth.json"),
            source_home.join("config.toml"),
            source_home.join("state_5.sqlite"),
            source_home.join("state_5.sqlite-wal"),
            source_home.join("state_5.sqlite-shm"),
        ];
        tracked_paths.extend(source_history_rollout_paths(source_home)?);
        tracked_paths.sort();
        tracked_paths.dedup();

        let temp_dir = TempDir::new()?;
        let mut snapshots = Vec::with_capacity(tracked_paths.len());
        for path in tracked_paths {
            snapshots.push(FileSnapshot::capture(temp_dir.path(), path)?);
        }

        Ok(Self {
            snapshots,
            temp_dir,
        })
    }

    fn restore(self) -> ModexResult<()> {
        let _ = self.temp_dir.path();
        for snapshot in self.snapshots {
            snapshot.restore()?;
        }
        Ok(())
    }
}

struct FileSnapshot {
    original_path: PathBuf,
    backup_path: PathBuf,
    existed: bool,
}

impl FileSnapshot {
    fn capture(backup_root: &Path, original_path: PathBuf) -> ModexResult<Self> {
        let sanitized = original_path
            .to_string_lossy()
            .replace('/', "__")
            .replace(':', "_");
        let backup_path = backup_root.join(sanitized);
        let existed = original_path.exists();
        if existed {
            fs::copy(&original_path, &backup_path)?;
        }
        Ok(Self {
            original_path,
            backup_path,
            existed,
        })
    }

    fn restore(self) -> ModexResult<()> {
        if self.existed {
            if let Some(parent) = self.original_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let temporary = self.original_path.with_file_name(format!(
                "{}.modex-restore",
                self.original_path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or("restore")
            ));
            fs::copy(&self.backup_path, &temporary)?;
            if let Ok(metadata) = fs::metadata(&self.original_path) {
                let _ = fs::set_permissions(&temporary, metadata.permissions());
            }
            fs::rename(temporary, &self.original_path)?;
        } else if self.original_path.exists() {
            fs::remove_file(&self.original_path)?;
        }
        Ok(())
    }
}

fn strip_managed_api_key_provider_config(
    existing: &str,
    remove_any_top_level_model_provider: bool,
) -> Vec<String> {
    let mut lines = Vec::new();
    let mut before_first_table = true;
    let mut skipping_managed_provider = false;
    for line in existing.lines() {
        let trimmed = line.trim();
        if is_table_header(trimmed) {
            before_first_table = false;
            if is_managed_provider_table(trimmed) {
                skipping_managed_provider = true;
                continue;
            }
            skipping_managed_provider = false;
        } else if skipping_managed_provider {
            continue;
        }
        if skipping_managed_provider || is_openai_base_url_line(line) {
            continue;
        }
        if before_first_table
            && is_model_provider_line(line)
            && (remove_any_top_level_model_provider || is_managed_model_provider_assignment(line))
        {
            continue;
        }
        lines.push(line.to_string());
    }
    tidy_config_lines(lines)
}

fn top_level_insert_index(lines: &[String]) -> usize {
    let mut index = lines
        .iter()
        .position(|line| is_table_header(line.trim()))
        .unwrap_or(lines.len());
    while index > 0 && lines[index - 1].trim().is_empty() {
        index -= 1;
    }
    index
}

fn tidy_config_lines(lines: Vec<String>) -> Vec<String> {
    let mut tidied = Vec::with_capacity(lines.len());
    for line in lines {
        if line.trim().is_empty()
            && tidied
                .last()
                .is_some_and(|last: &String| last.trim().is_empty())
        {
            continue;
        }
        tidied.push(line);
    }
    while tidied.first().is_some_and(|line| line.trim().is_empty()) {
        tidied.remove(0);
    }
    while tidied.last().is_some_and(|line| line.trim().is_empty()) {
        tidied.pop();
    }
    tidied
}

fn is_table_header(trimmed: &str) -> bool {
    trimmed.starts_with('[') && trimmed.ends_with(']')
}

fn is_managed_provider_table(trimmed: &str) -> bool {
    matches!(
        trimmed,
        "[model_providers.modex-api-key]" | "[model_providers.\"modex-api-key\"]"
    )
}

fn is_model_provider_line(line: &str) -> bool {
    is_toml_key_line(line, "model_provider")
}

fn is_managed_model_provider_assignment(line: &str) -> bool {
    line.split_once('=').is_some_and(|(key, value)| {
        key.trim() == "model_provider"
            && value.trim().trim_matches('"') == MODEX_API_KEY_PROVIDER_ID
    })
}

fn is_openai_base_url_line(line: &str) -> bool {
    is_toml_key_line(line, "openai_base_url")
}

fn is_toml_key_line(line: &str, key: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed == key
        || trimmed.starts_with(&format!("{key} "))
        || trimmed.starts_with(&format!("{key}="))
}

#[cfg(target_os = "macos")]
fn quit_codex_app_if_running(settings: &AppSettings) -> ModexResult<()> {
    let output = Command::new("osascript")
        .arg("-e")
        .arg(macos_quit_codex_app_script(&settings.app_name))
        .output()?;
    if output.status.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let mut message =
        "Codex 未退出，账号切换已取消。请等当前任务结束，或在 Codex 中确认退出后再试。".to_string();
    if !detail.is_empty() {
        message.push_str(&format!(" ({detail})"));
    }
    Err(ModexError::from(message))
}

#[cfg(not(target_os = "macos"))]
fn quit_codex_app_if_running(_settings: &AppSettings) -> ModexResult<()> {
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn macos_quit_codex_app_script(app_name: &str) -> String {
    let app_name = escape_applescript_string(app_name);
    format!(
        r#"if application "{app_name}" is running then
	tell application "{app_name}" to quit
	repeat with attempt from 1 to 50
		if application "{app_name}" is not running then exit repeat
		delay 0.1
	end repeat
	if application "{app_name}" is running then
		error "{app_name} did not quit" number -128
	end if
end if"#
    )
}

pub fn open_codex_app_launch_command(settings: &AppSettings) -> ProgramInvocation {
    #[cfg(target_os = "macos")]
    {
        return ProgramInvocation {
            program: PathBuf::from("open"),
            args: vec!["-a".to_string(), settings.app_name.clone()],
            envs: Vec::new(),
        };
    }

    #[cfg(not(target_os = "macos"))]
    {
        ProgramInvocation {
            program: resolve_codex_binary(&settings.codex_binary),
            args: vec!["app".to_string()],
            envs: build_codex_env(&settings.source_home),
        }
    }
}

fn spawn_program(invocation: ProgramInvocation) -> ModexResult<()> {
    Command::new(invocation.program)
        .args(invocation.args)
        .envs(invocation.envs)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

pub fn activate_codex_app(settings: &AppSettings) -> ModexResult<()> {
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg("-a")
            .arg(&settings.app_name)
            .spawn()?;
        return Ok(());
    }
    #[cfg(not(target_os = "macos"))]
    {
        Command::new(resolve_codex_binary(&settings.codex_binary))
            .arg("app")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        Ok(())
    }
}

pub fn read_quota_snapshot(
    settings: &AppSettings,
    identity: &AppIdentity,
) -> ModexResult<QuotaSnapshot> {
    let temp_home = temporary_identity_home(identity)?;
    let payload = request_rate_limits(
        &settings.codex_binary,
        temp_home.path(),
        Duration::from_secs(30),
    )?;
    snapshot_from_rate_limits(&identity.name, &payload)
}

pub fn read_account_display_name(
    settings: &AppSettings,
    identity: &AppIdentity,
) -> ModexResult<Option<String>> {
    let temp_home = temporary_identity_home(identity)?;
    let payload = request_app_server(
        &settings.codex_binary,
        temp_home.path(),
        "account/read",
        serde_json::json!({ "refreshToken": false }),
        Duration::from_secs(30),
    )?;
    Ok(account_display_name_from_response(&payload))
}

pub fn account_display_name_from_response(payload: &Value) -> Option<String> {
    let account = payload.get("account")?;
    match account.get("type").and_then(Value::as_str)? {
        "chatgpt" => {
            let email = account.get("email").and_then(Value::as_str)?.trim();
            if email.is_empty() {
                return None;
            }
            let plan = plan_label(account.get("planType").and_then(Value::as_str));
            if plan == "计划未知" {
                Some(email.to_string())
            } else {
                Some(format!("{email} · {plan}"))
            }
        }
        "amazonBedrock" => Some("Amazon Bedrock".to_string()),
        "apiKey" => None,
        _ => None,
    }
}

pub fn request_rate_limits(
    codex_binary: &str,
    codex_home: &Path,
    timeout: Duration,
) -> ModexResult<Value> {
    request_app_server(
        codex_binary,
        codex_home,
        "account/rateLimits/read",
        Value::Null,
        timeout,
    )
}

pub fn request_app_server(
    codex_binary: &str,
    codex_home: &Path,
    method: &str,
    params: Value,
    timeout: Duration,
) -> ModexResult<Value> {
    let mut child = Command::new(resolve_codex_binary(codex_binary))
        .arg("app-server")
        .arg("--listen")
        .arg("stdio://")
        .envs(build_codex_env(codex_home))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| ModexError::from("app-server stdin is unavailable"))?;
    write_request(&mut stdin, 1, "initialize", initialize_params())?;
    write_request(&mut stdin, 2, method, params)?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ModexError::from("app-server stdout is unavailable"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| ModexError::from("app-server stderr is unavailable"))?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(message) = serde_json::from_str::<Value>(&line) {
                if message.get("id").and_then(Value::as_i64) == Some(2) {
                    let _ = tx.send(message);
                    return;
                }
            }
        }
    });
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            eprintln!("codex app-server: {line}");
        }
    });

    let message = rx
        .recv_timeout(timeout)
        .map_err(|_| ModexError::from(format!("timed out waiting for {method}")))?;
    let _ = child.kill();
    let _ = child.wait();
    if let Some(error) = message.get("error") {
        return Err(ModexError::from(error.to_string()));
    }
    message
        .get("result")
        .cloned()
        .ok_or_else(|| ModexError::from(format!("{method} returned no result")))
}

pub fn build_codex_env(codex_home: &Path) -> Vec<(String, String)> {
    vec![("CODEX_HOME".to_string(), codex_home.display().to_string())]
}

fn temporary_identity_home(identity: &AppIdentity) -> ModexResult<TempDir> {
    let auth_file = identity.codex_home.join("auth.json");
    if !auth_file.exists() {
        return Err(ModexError::from(format!(
            "账号缺少登录凭据：{}",
            auth_file.display()
        )));
    }
    let temp_home = tempfile::Builder::new().prefix("modex-auth-").tempdir()?;
    std::fs::copy(&auth_file, temp_home.path().join("auth.json"))?;
    apply_openai_base_url_config(temp_home.path(), identity.api_base_url.as_deref())?;
    Ok(temp_home)
}

fn write_request(stdin: &mut impl Write, id: u8, method: &str, params: Value) -> ModexResult<()> {
    let request = serde_json::json!({
        "id": id,
        "method": method,
        "params": params
    });
    writeln!(stdin, "{request}")?;
    stdin.flush()?;
    Ok(())
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

pub fn resolve_codex_binary(codex_binary: &str) -> PathBuf {
    resolve_codex_binary_with(
        codex_binary,
        |value| path_lookup(value),
        &[PathBuf::from(DEFAULT_CODEX_APP_CLI)],
    )
}

pub fn resolve_codex_binary_with(
    configured: &str,
    which: impl Fn(&str) -> Option<PathBuf>,
    app_cli_paths: &[PathBuf],
) -> PathBuf {
    let value = configured.trim();
    let value = if value.is_empty() { "codex" } else { value };
    if value.contains('/') || value.contains('\\') {
        return expand_home(value);
    }
    if let Some(path_match) = which(value) {
        return path_match;
    }
    if value == "codex" {
        for candidate in app_cli_paths {
            if candidate.exists() {
                return candidate.clone();
            }
        }
    }
    PathBuf::from(value)
}

fn path_lookup(binary: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join(binary);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn expand_home(value: &str) -> PathBuf {
    if value == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(value));
    }
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(value)
}

fn escape_config_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    use super::super::app_config::{AppIdentity, AppSettings};
    use super::super::ModexError;
    use super::*;

    #[test]
    fn macos_switch_does_not_sync_auth_or_launch_when_codex_refuses_to_quit() {
        let events = RefCell::new(Vec::new());
        let settings = AppSettings::default_for_home(PathBuf::from("/tmp/modex-test"));
        let identity = identity_at("/tmp/modex-test/.modex/new");

        let result = open_codex_app_with_operations(
            &settings,
            &identity,
            |_| {
                events.borrow_mut().push("quit");
                Err(ModexError::from("quit canceled"))
            },
            |_settings, _identity| {
                events.borrow_mut().push("prepare");
                Ok(())
            },
            |_invocation| {
                events.borrow_mut().push("launch");
                Ok(())
            },
        );

        assert!(result.is_err());
        assert_eq!(events.into_inner(), vec!["quit"]);
    }

    #[test]
    fn macos_switch_syncs_auth_only_after_codex_has_quit() {
        let events = RefCell::new(Vec::new());
        let settings = AppSettings::default_for_home(PathBuf::from("/tmp/modex-test"));
        let identity = identity_at("/tmp/modex-test/.modex/new");

        open_codex_app_with_operations(
            &settings,
            &identity,
            |_| {
                events.borrow_mut().push("quit");
                Ok(())
            },
            |_settings, _identity| {
                events.borrow_mut().push("prepare");
                Ok(())
            },
            |_invocation| {
                events.borrow_mut().push("launch");
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(events.into_inner(), vec!["quit", "prepare", "launch"]);
    }

    fn identity_at(path: &str) -> AppIdentity {
        AppIdentity {
            name: "New".to_string(),
            codex_home: Path::new(path).to_path_buf(),
            monitor: true,
            workspace_id: None,
            auth_type: Default::default(),
            api_base_url: None,
        }
    }
}

#[cfg(target_os = "macos")]
fn escape_applescript_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

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
use super::sync::sync_identity_auth;
use super::{ModexError, ModexResult};

const DEFAULT_CODEX_APP_CLI: &str = "/Applications/Codex.app/Contents/Resources/codex";

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
        sync_identity_auth,
        spawn_program,
    )
}

fn open_codex_app_with_operations(
    settings: &AppSettings,
    identity: &AppIdentity,
    quit: impl FnOnce(&AppSettings) -> ModexResult<()>,
    sync: impl FnOnce(&Path, &Path) -> ModexResult<PathBuf>,
    launch: impl FnOnce(ProgramInvocation) -> ModexResult<()>,
) -> ModexResult<()> {
    #[cfg(target_os = "macos")]
    quit(settings)?;
    #[cfg(not(target_os = "macos"))]
    let _ = quit;
    sync(&settings.source_home, &identity.codex_home)?;
    apply_identity_runtime_config(settings, identity)?;
    launch(open_codex_app_launch_command(settings))
}

pub fn prepare_identity_for_launch(
    settings: &AppSettings,
    identity: &AppIdentity,
) -> ModexResult<()> {
    sync_identity_auth(&settings.source_home, &identity.codex_home)?;
    apply_identity_runtime_config(settings, identity)
}

pub fn apply_identity_runtime_config(
    settings: &AppSettings,
    identity: &AppIdentity,
) -> ModexResult<()> {
    apply_openai_base_url_config(&settings.source_home, identity.api_base_url.as_deref())
}

pub fn apply_openai_base_url_config(codex_home: &Path, base_url: Option<&str>) -> ModexResult<()> {
    std::fs::create_dir_all(codex_home)?;
    let config_path = codex_home.join("config.toml");
    let existing = std::fs::read_to_string(&config_path).unwrap_or_default();
    let mut lines = existing
        .lines()
        .filter(|line| !is_openai_base_url_line(line))
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if let Some(base_url) = base_url.map(str::trim).filter(|value| !value.is_empty()) {
        lines.push(format!(
            "openai_base_url = \"{}\"",
            escape_config_value(base_url)
        ));
    }
    let next = if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    };
    std::fs::write(config_path, next)?;
    Ok(())
}

fn is_openai_base_url_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed == "openai_base_url"
        || trimmed.starts_with("openai_base_url ")
        || trimmed.starts_with("openai_base_url=")
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
            |_source, _identity| {
                events.borrow_mut().push("sync");
                Ok(PathBuf::from("/tmp/modex-test/source/auth.json"))
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
            |_source, _identity| {
                events.borrow_mut().push("sync");
                Ok(PathBuf::from("/tmp/modex-test/source/auth.json"))
            },
            |_invocation| {
                events.borrow_mut().push("launch");
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(events.into_inner(), vec!["quit", "sync", "launch"]);
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

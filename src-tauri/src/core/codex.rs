use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::Value;
use tempfile::TempDir;

use super::app_config::{AppIdentity, AppSettings};
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

pub fn open_codex_app(settings: &AppSettings, identity: &AppIdentity) -> ModexResult<()> {
    sync_identity_auth(&settings.source_home, &identity.codex_home)?;
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("osascript")
            .arg("-e")
            .arg(macos_quit_codex_app_script(&settings.app_name))
            .status();
    }
    spawn_program(open_codex_app_launch_command(settings))?;
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn macos_quit_codex_app_script(app_name: &str) -> String {
    let app_name = escape_applescript_string(app_name);
    format!(
        r#"if application "{app_name}" is running then
	tell application "{app_name}" to quit
	repeat with _attempt in 1 to 50
		if application "{app_name}" is not running then exit repeat
		delay 0.1
	end repeat
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
    let temp_home = temporary_auth_home(&identity.codex_home)?;
    let payload = request_rate_limits(
        &settings.codex_binary,
        temp_home.path(),
        Duration::from_secs(30),
    )?;
    snapshot_from_rate_limits(&identity.name, &payload)
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

fn temporary_auth_home(identity_home: &Path) -> ModexResult<TempDir> {
    let auth_file = identity_home.join("auth.json");
    if !auth_file.exists() {
        return Err(ModexError::from(format!(
            "账号缺少登录凭据：{}",
            auth_file.display()
        )));
    }
    let temp_home = tempfile::Builder::new().prefix("modex-auth-").tempdir()?;
    std::fs::copy(&auth_file, temp_home.path().join("auth.json"))?;
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

#[cfg(target_os = "macos")]
fn escape_applescript_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

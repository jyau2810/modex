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
    history_sync_provider_for_identity, sync_identity_auth, sync_source_history_provider,
};
use super::{ModexError, ModexResult};

const DEFAULT_CODEX_APP_CLI: &str = "/Applications/Codex.app/Contents/Resources/codex";
const MODEX_API_KEY_PROVIDER_ID: &str = "modex-api-key";
const MODEX_API_KEY_PROVIDER_NAME: &str = "Modex API Key";
const CODEX_PLUGIN_AUTH_GATE_ORIGINAL: &[u8] =
    b"function e(e){return e!==`chatgpt`}export{e as t};";
const CODEX_PLUGIN_AUTH_GATE_PATCHED: &[u8] = b"function e(e){return false        }export{e as t};";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramInvocation {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub envs: Vec<(String, String)>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrepareIdentityOutcome {
    pub history_warning: Option<String>,
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

pub fn open_codex_app(
    settings: &AppSettings,
    identity: &AppIdentity,
) -> ModexResult<PrepareIdentityOutcome> {
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
    prepare: impl FnOnce(&AppSettings, &AppIdentity) -> ModexResult<PrepareIdentityOutcome>,
    launch: impl FnOnce(ProgramInvocation) -> ModexResult<()>,
) -> ModexResult<PrepareIdentityOutcome> {
    #[cfg(target_os = "macos")]
    quit(settings)?;
    #[cfg(not(target_os = "macos"))]
    let _ = quit;
    let outcome = prepare(settings, identity)?;
    launch(open_codex_app_launch_command(settings))?;
    Ok(outcome)
}

pub fn prepare_identity_for_launch(
    settings: &AppSettings,
    identity: &AppIdentity,
) -> ModexResult<PrepareIdentityOutcome> {
    sync_identity_auth(&settings.source_home, &identity.codex_home)?;
    sync_plugin_registration_config(&settings.source_home, &identity.codex_home)?;
    apply_identity_runtime_config(settings, identity)?;
    let mut outcome = PrepareIdentityOutcome::default();
    let provider = history_sync_provider_for_identity(identity);
    match sync_source_history_provider(&settings.source_home, provider) {
        Ok(history_outcome) => {
            if let Some(warning) = history_outcome.encrypted_content_warning {
                eprintln!("Modex history provider sync warning: {warning}");
                outcome.history_warning = Some(warning);
            }
        }
        Err(error) => {
            eprintln!("Modex history provider sync skipped: {error}");
        }
    }
    Ok(outcome)
}

pub fn apply_identity_runtime_config(
    settings: &AppSettings,
    identity: &AppIdentity,
) -> ModexResult<()> {
    apply_openai_base_url_config(&settings.source_home, identity.api_base_url.as_deref())
}

pub fn sync_plugin_registration_config(
    source_home: &Path,
    identity_home: &Path,
) -> ModexResult<()> {
    let source_config = source_home.join("config.toml");
    let identity_config = identity_home.join("config.toml");
    let source_existing = std::fs::read_to_string(&source_config).unwrap_or_default();
    let identity_existing = std::fs::read_to_string(&identity_config).unwrap_or_default();
    let source_sections = plugin_registration_sections(&source_existing);
    let identity_sections = plugin_registration_sections(&identity_existing);

    let next_source = merge_missing_plugin_sections(&source_existing, &identity_sections);
    if next_source != source_existing {
        std::fs::create_dir_all(source_home)?;
        std::fs::write(&source_config, next_source)?;
    }

    let next_identity = merge_missing_plugin_sections(&identity_existing, &source_sections);
    if next_identity != identity_existing {
        std::fs::create_dir_all(identity_home)?;
        std::fs::write(&identity_config, next_identity)?;
    }

    Ok(())
}

pub fn apply_openai_base_url_config(codex_home: &Path, base_url: Option<&str>) -> ModexResult<()> {
    std::fs::create_dir_all(codex_home)?;
    let config_path = codex_home.join("config.toml");
    let existing = std::fs::read_to_string(&config_path).unwrap_or_default();
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
    std::fs::write(config_path, next)?;
    Ok(())
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

#[derive(Clone, Debug)]
struct ConfigTableSection {
    header: String,
    lines: Vec<String>,
}

fn plugin_registration_sections(existing: &str) -> Vec<ConfigTableSection> {
    let mut sections = Vec::new();
    let mut current_header: Option<String> = None;
    let mut current_lines: Vec<String> = Vec::new();

    for line in existing.lines() {
        let trimmed = line.trim();
        if is_table_header(trimmed) {
            if let Some(header) = current_header.take() {
                sections.push(ConfigTableSection {
                    header,
                    lines: trim_trailing_blank_lines(current_lines),
                });
                current_lines = Vec::new();
            }
            if is_plugin_registration_table(trimmed) {
                current_header = Some(trimmed.to_string());
                current_lines.push(line.to_string());
            }
            continue;
        }
        if current_header.is_some() {
            current_lines.push(line.to_string());
        }
    }

    if let Some(header) = current_header {
        sections.push(ConfigTableSection {
            header,
            lines: trim_trailing_blank_lines(current_lines),
        });
    }

    sections
}

fn merge_missing_plugin_sections(existing: &str, incoming: &[ConfigTableSection]) -> String {
    let mut lines = existing
        .lines()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let mut existing_headers = plugin_registration_sections(existing)
        .into_iter()
        .map(|section| section.header)
        .collect::<std::collections::HashSet<_>>();

    for section in incoming {
        if !existing_headers.insert(section.header.clone()) {
            continue;
        }
        while lines.last().is_some_and(|line| line.trim().is_empty()) {
            lines.pop();
        }
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.extend(section.lines.clone());
    }

    if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    }
}

fn trim_trailing_blank_lines(mut lines: Vec<String>) -> Vec<String> {
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    lines
}

fn is_plugin_registration_table(trimmed: &str) -> bool {
    trimmed.starts_with("[plugins.") || trimmed.starts_with("[marketplaces.")
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

#[cfg(target_os = "macos")]
pub fn patch_codex_plugin_auth_gate_and_restart(settings: &AppSettings) -> ModexResult<()> {
    let app_bundle = macos_codex_app_bundle_path(&settings.app_name);
    if !app_bundle.exists() {
        return Err(ModexError::from(format!(
            "未找到 Codex App：{}",
            app_bundle.display()
        )));
    }

    let asar_path = app_bundle.join("Contents/Resources/app.asar");
    if !asar_path.is_file() {
        return Err(ModexError::from(format!(
            "未找到 Codex app.asar：{}",
            asar_path.display()
        )));
    }

    quit_codex_app_if_running(settings)?;
    let _changed = patch_plugin_auth_gate_file(&asar_path)?;
    run_checked_invocation(
        electron_fuses_disable_asar_integrity_invocation(&app_bundle),
        "关闭 Electron asar 完整性校验失败",
    )?;
    run_checked_invocation(
        ad_hoc_codesign_app_invocation(&app_bundle),
        "重新签名 Codex App 失败",
    )?;
    spawn_program(ProgramInvocation {
        program: PathBuf::from("open"),
        args: vec![app_bundle.display().to_string()],
        envs: Vec::new(),
    })?;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn patch_codex_plugin_auth_gate_and_restart(_settings: &AppSettings) -> ModexResult<()> {
    Err(ModexError::from(
        "Codex App 插件入口 patch 目前只支持 macOS。",
    ))
}

#[cfg(target_os = "macos")]
fn macos_codex_app_bundle_path(app_name: &str) -> PathBuf {
    let app_name = app_name.trim();
    if app_name.contains('/') {
        return PathBuf::from(app_name);
    }
    let app_name = app_name.strip_suffix(".app").unwrap_or(app_name);
    PathBuf::from("/Applications").join(format!("{app_name}.app"))
}

fn patch_plugin_auth_gate_file(path: &Path) -> ModexResult<bool> {
    let mut bytes = std::fs::read(path)?;
    let changed = patch_plugin_auth_gate_bytes(&mut bytes)?;
    if changed {
        std::fs::write(path, bytes)?;
    }
    Ok(changed)
}

fn patch_plugin_auth_gate_bytes(bytes: &mut [u8]) -> ModexResult<bool> {
    debug_assert_eq!(
        CODEX_PLUGIN_AUTH_GATE_ORIGINAL.len(),
        CODEX_PLUGIN_AUTH_GATE_PATCHED.len()
    );
    let original_count = count_subslice(bytes, CODEX_PLUGIN_AUTH_GATE_ORIGINAL);
    let patched_count = count_subslice(bytes, CODEX_PLUGIN_AUTH_GATE_PATCHED);

    match (original_count, patched_count) {
        (1, 0) => {
            let offset = find_subslice(bytes, CODEX_PLUGIN_AUTH_GATE_ORIGINAL)
                .ok_or_else(|| ModexError::from("未找到 Codex 插件入口判断逻辑"))?;
            bytes[offset..offset + CODEX_PLUGIN_AUTH_GATE_PATCHED.len()]
                .copy_from_slice(CODEX_PLUGIN_AUTH_GATE_PATCHED);
            Ok(true)
        }
        (0, 1) => Ok(false),
        (0, 0) => Err(ModexError::from(
            "未找到可 patch 的 Codex 插件入口逻辑，当前 Codex 版本可能已变更。",
        )),
        _ => Err(ModexError::from(
            "Codex 插件入口逻辑匹配结果不唯一，已取消 patch。",
        )),
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn count_subslice(haystack: &[u8], needle: &[u8]) -> usize {
    if needle.is_empty() {
        return 0;
    }
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

#[cfg(target_os = "macos")]
fn electron_fuses_disable_asar_integrity_invocation(app_bundle: &Path) -> ProgramInvocation {
    let app_bundle = shell_quote(&app_bundle.to_string_lossy());
    ProgramInvocation {
        program: PathBuf::from("/bin/zsh"),
        args: vec![
            "-lc".to_string(),
            format!(
                "npx --yes @electron/fuses write --app {app_bundle} EnableEmbeddedAsarIntegrityValidation=off"
            ),
        ],
        envs: Vec::new(),
    }
}

#[cfg(target_os = "macos")]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r#"'\''"#))
}

#[cfg(target_os = "macos")]
fn ad_hoc_codesign_app_invocation(app_bundle: &Path) -> ProgramInvocation {
    ProgramInvocation {
        program: PathBuf::from("codesign"),
        args: vec![
            "--force".to_string(),
            "--deep".to_string(),
            "--sign".to_string(),
            "-".to_string(),
            app_bundle.display().to_string(),
        ],
        envs: Vec::new(),
    }
}

#[cfg(target_os = "macos")]
fn run_checked_invocation(invocation: ProgramInvocation, context: &str) -> ModexResult<()> {
    let output = Command::new(&invocation.program)
        .args(&invocation.args)
        .envs(invocation.envs)
        .output()?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if stderr.is_empty() { stdout } else { stderr };
    if detail.is_empty() {
        Err(ModexError::from(context.to_string()))
    } else {
        Err(ModexError::from(format!("{context}：{detail}")))
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
    fn macos_switch_does_not_prepare_or_launch_when_codex_refuses_to_quit() {
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
                Ok(PrepareIdentityOutcome::default())
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
    fn macos_switch_prepares_identity_only_after_codex_has_quit() {
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
                Ok(PrepareIdentityOutcome::default())
            },
            |_invocation| {
                events.borrow_mut().push("launch");
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(events.into_inner(), vec!["quit", "prepare", "launch"]);
    }

    #[test]
    fn plugin_auth_gate_patch_replaces_original_gate_once() {
        let mut bytes = b"before function e(e){return e!==`chatgpt`}export{e as t}; after".to_vec();

        assert_eq!(patch_plugin_auth_gate_bytes(&mut bytes).unwrap(), true);

        let next = String::from_utf8(bytes).unwrap();
        assert!(!next.contains("function e(e){return e!==`chatgpt`}export{e as t};"));
        assert!(next.contains("function e(e){return false        }export{e as t};"));
    }

    #[test]
    fn plugin_auth_gate_patch_is_idempotent() {
        let mut bytes = b"before function e(e){return false        }export{e as t}; after".to_vec();

        assert_eq!(patch_plugin_auth_gate_bytes(&mut bytes).unwrap(), false);
    }

    #[test]
    fn disable_asar_integrity_invocation_keeps_app_asar_loading_enabled() {
        let invocation = electron_fuses_disable_asar_integrity_invocation(Path::new(
            "/Applications/Codex App.app",
        ));

        assert_eq!(invocation.program, PathBuf::from("/bin/zsh"));
        assert_eq!(
            invocation.args,
            vec![
                "-lc",
                "npx --yes @electron/fuses write --app '/Applications/Codex App.app' EnableEmbeddedAsarIntegrityValidation=off"
            ]
        );
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

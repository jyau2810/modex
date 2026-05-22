use assert_fs::prelude::*;
use modex_lib::core::app_config::{AppIdentity, AppSettings, IdentityAuthType};
use modex_lib::core::codex::{
    account_display_name_from_response, api_key_login_invocation, apply_openai_base_url_config,
    open_codex_app_launch_command, prepare_identity_for_launch, resolve_codex_binary_with,
    ProgramInvocation,
};
use rusqlite::{params, Connection};

#[cfg(target_os = "macos")]
use modex_lib::core::codex::macos_quit_codex_app_script;

#[test]
fn resolves_codex_to_app_cli_when_path_lookup_fails() {
    let temp = assert_fs::TempDir::new().unwrap();
    let app_cli = temp.child("Codex.app/Contents/Resources/codex");
    app_cli.touch().unwrap();

    let resolved = resolve_codex_binary_with("codex", |_| None, &[app_cli.path().to_path_buf()]);

    assert_eq!(resolved, app_cli.path().to_path_buf());
}

#[test]
fn resolves_configured_path_without_path_lookup() {
    let resolved = resolve_codex_binary_with("~/bin/codex", |_| Some("/wrong/codex".into()), &[]);

    assert!(resolved.ends_with("bin/codex"));
}

#[test]
fn api_key_login_command_reads_key_from_stdin() {
    let settings = AppSettings::default_for_home("/tmp/modex-test".into());
    let identity = AppIdentity {
        name: "API".to_string(),
        codex_home: "/tmp/modex-test/.modex/api".into(),
        monitor: false,
        workspace_id: None,
        auth_type: IdentityAuthType::ApiKey,
        api_base_url: None,
    };

    let invocation: ProgramInvocation = api_key_login_invocation(&settings, &identity);

    assert_eq!(
        invocation.args,
        vec!["login".to_string(), "--with-api-key".to_string()]
    );
    assert_eq!(
        invocation.envs,
        vec![(
            "CODEX_HOME".to_string(),
            "/tmp/modex-test/.modex/api".to_string()
        )]
    );
}

#[test]
fn account_display_name_uses_chatgpt_email_and_ignores_api_key_accounts() {
    let chatgpt_name = account_display_name_from_response(&serde_json::json!({
        "account": {
            "type": "chatgpt",
            "email": "project@example.com",
            "planType": "team"
        }
    }));
    let api_key_name = account_display_name_from_response(&serde_json::json!({
        "account": {
            "type": "apiKey"
        }
    }));

    assert_eq!(
        chatgpt_name.as_deref(),
        Some("project@example.com · 团队版")
    );
    assert_eq!(api_key_name, None);
}

#[test]
fn apply_openai_base_url_config_sets_or_removes_managed_provider() {
    let temp = assert_fs::TempDir::new().unwrap();
    let config = temp.child("config.toml");
    config
        .write_str(
            "model = \"gpt-5.2\"\nopenai_base_url = \"https://old.example/v1\"\n\n[projects.\"/tmp/project\"]\ntrust_level = \"trusted\"\n\n[model_providers.modex-api-key]\nname = \"Old Modex Provider\"\nbase_url = \"https://old.example/v1\"\nwire_api = \"responses\"\nrequires_openai_auth = true\nsupports_websockets = false\n\n[mcp_servers.test]\ncommand = \"test\"\n",
        )
        .unwrap();

    apply_openai_base_url_config(temp.path(), Some("https://sub2api.flatincbr.com")).unwrap();
    assert_eq!(
        std::fs::read_to_string(config.path()).unwrap(),
        "model = \"gpt-5.2\"\nmodel_provider = \"modex-api-key\"\n\n[projects.\"/tmp/project\"]\ntrust_level = \"trusted\"\n\n[mcp_servers.test]\ncommand = \"test\"\n\n[model_providers.modex-api-key]\nname = \"Modex API Key\"\nbase_url = \"https://sub2api.flatincbr.com\"\nwire_api = \"responses\"\nrequires_openai_auth = true\nsupports_websockets = false\n"
    );

    apply_openai_base_url_config(temp.path(), None).unwrap();
    assert_eq!(
        std::fs::read_to_string(config.path()).unwrap(),
        "model = \"gpt-5.2\"\n\n[projects.\"/tmp/project\"]\ntrust_level = \"trusted\"\n\n[mcp_servers.test]\ncommand = \"test\"\n"
    );
}

#[test]
fn prepare_identity_for_launch_syncs_api_key_auth_and_applies_base_url() {
    let temp = assert_fs::TempDir::new().unwrap();
    let source_home = temp.path().join("source");
    let api_home = temp.path().join(".modex/api");
    std::fs::create_dir_all(&source_home).unwrap();
    std::fs::create_dir_all(&api_home).unwrap();
    std::fs::create_dir_all(source_home.join("sessions")).unwrap();
    std::fs::write(source_home.join("config.toml"), "model = \"gpt-5.2\"\n").unwrap();
    std::fs::write(
        api_home.join("auth.json"),
        r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-test"}"#,
    )
    .unwrap();
    std::fs::write(
        source_home.join("sessions/thread-a.jsonl"),
        format!(
            "{}\n",
            serde_json::json!({
                "session_meta": {
                    "payload": {
                        "model_provider": "openai"
                    }
                }
            })
        ),
    )
    .unwrap();
    create_threads_db(
        &source_home.join("state_5.sqlite"),
        &[("thread-a", "openai", "sessions/thread-a.jsonl", 0_i64)],
    );

    let mut settings = AppSettings::default_for_home(temp.path().to_path_buf());
    settings.source_home = source_home.clone();
    let identity = AppIdentity {
        name: "Gateway".to_string(),
        codex_home: api_home,
        monitor: false,
        workspace_id: None,
        auth_type: IdentityAuthType::ApiKey,
        api_base_url: Some("https://gateway.example/v1".to_string()),
    };

    prepare_identity_for_launch(&settings, &identity).unwrap();

    assert_eq!(
        std::fs::read_to_string(source_home.join("auth.json")).unwrap(),
        r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-test"}"#
    );
    assert_eq!(
        std::fs::read_to_string(source_home.join("config.toml")).unwrap(),
        "model = \"gpt-5.2\"\nmodel_provider = \"modex-api-key\"\n\n[model_providers.modex-api-key]\nname = \"Modex API Key\"\nbase_url = \"https://gateway.example/v1\"\nwire_api = \"responses\"\nrequires_openai_auth = true\nsupports_websockets = false\n"
    );
    let connection = Connection::open(source_home.join("state_5.sqlite")).unwrap();
    let provider: String = connection
        .query_row(
            "SELECT model_provider FROM threads WHERE id = 'thread-a'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(provider, "modex-api-key");
    let first_line = std::fs::read_to_string(source_home.join("sessions/thread-a.jsonl"))
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_string();
    let payload: serde_json::Value = serde_json::from_str(&first_line).unwrap();
    assert_eq!(
        payload["session_meta"]["payload"]["model_provider"],
        "openai"
    );
}

#[test]
fn prepare_identity_for_launch_rolls_back_runtime_files_when_history_sync_fails() {
    let temp = assert_fs::TempDir::new().unwrap();
    let source_home = temp.path().join("source");
    let api_home = temp.path().join(".modex/api");
    std::fs::create_dir_all(&source_home).unwrap();
    std::fs::create_dir_all(&api_home).unwrap();
    std::fs::write(source_home.join("auth.json"), r#"{"old":true}"#).unwrap();
    std::fs::write(source_home.join("config.toml"), "model = \"gpt-5.2\"\n").unwrap();
    std::fs::write(source_home.join("state_5.sqlite"), "not-a-sqlite-db").unwrap();
    std::fs::write(
        api_home.join("auth.json"),
        r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-test"}"#,
    )
    .unwrap();

    let mut settings = AppSettings::default_for_home(temp.path().to_path_buf());
    settings.source_home = source_home.clone();
    let identity = AppIdentity {
        name: "Gateway".to_string(),
        codex_home: api_home,
        monitor: false,
        workspace_id: None,
        auth_type: IdentityAuthType::ApiKey,
        api_base_url: Some("https://gateway.example/v1".to_string()),
    };

    let error = prepare_identity_for_launch(&settings, &identity).unwrap_err();

    let error_text = error.to_string().to_ascii_lowercase();
    assert!(error_text.contains("database") || error_text.contains("sqlite"));
    assert_eq!(
        std::fs::read_to_string(source_home.join("auth.json")).unwrap(),
        r#"{"old":true}"#
    );
    assert_eq!(
        std::fs::read_to_string(source_home.join("config.toml")).unwrap(),
        "model = \"gpt-5.2\"\n"
    );
    assert_eq!(
        std::fs::read_to_string(source_home.join("state_5.sqlite")).unwrap(),
        "not-a-sqlite-db"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn macos_switch_launches_codex_app_without_creating_a_project() {
    let temp = assert_fs::TempDir::new().unwrap();
    let settings = AppSettings::default_for_home(temp.path().to_path_buf());

    let command = open_codex_app_launch_command(&settings);

    assert_eq!(command.program.to_string_lossy(), "open");
    assert_eq!(command.args, vec!["-a".to_string(), "Codex".to_string()]);
    assert!(command.envs.is_empty());
}

#[cfg(target_os = "macos")]
#[test]
fn macos_quit_script_waits_until_codex_has_stopped_before_reopening() {
    let script = macos_quit_codex_app_script("Codex");

    assert!(script.contains(r#"if application "Codex" is running then"#));
    assert!(script.contains(r#"tell application "Codex" to quit"#));
    assert!(script.contains("repeat with attempt from 1 to 50"));
    assert!(script.contains(r#"if application "Codex" is not running then exit repeat"#));
    assert!(script.contains("delay 0.1"));
    assert!(script.contains(r#"error "Codex did not quit" number -128"#));
}

#[cfg(target_os = "macos")]
#[test]
fn macos_quit_script_is_valid_applescript() {
    let script = format!(
        "if false then\n{}\nend if",
        macos_quit_codex_app_script("Finder")
    );

    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .expect("osascript should be available on macOS");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn create_threads_db(path: &std::path::Path, rows: &[(&str, &str, &str, i64)]) {
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                cwd TEXT,
                rollout_path TEXT NOT NULL,
                model_provider TEXT,
                archived INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER,
                updated_at INTEGER
            );",
        )
        .unwrap();
    for (id, provider, rollout_path, archived) in rows {
        connection
            .execute(
                "INSERT INTO threads (
                    id,
                    title,
                    cwd,
                    rollout_path,
                    model_provider,
                    archived,
                    created_at,
                    updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1710000000, 1710000000)",
                params![
                    id,
                    format!("Thread {id}"),
                    format!("/tmp/{id}"),
                    rollout_path,
                    provider,
                    archived
                ],
            )
            .unwrap();
    }
}

use std::fs;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use assert_fs::prelude::*;
use modex_lib::core::app_config::{AppIdentity, AppSettings, IdentityAuthType};
use modex_lib::core::codex::prepare_identity_for_launch;
use modex_lib::core::sync::{
    history_sync_provider_for_identity, sync_source_history_provider, HistorySyncProvider,
};
use rusqlite::{params, Connection};

#[test]
fn history_sync_provider_keeps_plain_openai_api_keys_on_openai() {
    let chatgpt = identity(IdentityAuthType::ChatGpt, None);
    let openai_api_key = identity(IdentityAuthType::ApiKey, None);
    let relay_api_key = identity(IdentityAuthType::ApiKey, Some("https://gateway.example/v1"));

    assert_eq!(
        history_sync_provider_for_identity(&chatgpt),
        HistorySyncProvider::OpenAi
    );
    assert_eq!(
        history_sync_provider_for_identity(&openai_api_key),
        HistorySyncProvider::OpenAi
    );
    assert_eq!(
        history_sync_provider_for_identity(&relay_api_key),
        HistorySyncProvider::ModexApiKey
    );
}

#[test]
fn sync_source_history_provider_repairs_rollout_and_sqlite_visibility_metadata() {
    let temp = assert_fs::TempDir::new().unwrap();
    let source_home = temp.child("source");
    let sessions_path = source_home.child("sessions/2026/05/28");
    sessions_path.create_dir_all().unwrap();
    source_home.create_dir_all().unwrap();

    let rollout = sessions_path.child("rollout-2026-05-28T10-00-00-thread-open.jsonl");
    rollout
        .write_str(&current_rollout_jsonl(
            "openai",
            "thread-open",
            "/tmp/project-open",
            true,
        ))
        .unwrap();
    let original_mtime = UNIX_EPOCH + Duration::from_secs(1_714_000_000);
    set_mtime(rollout.path(), original_mtime);
    source_home
        .child(".codex-global-state.json")
        .write_str(
            &serde_json::json!({
                "electron-saved-workspace-roots": ["/tmp/project-open"],
                "project-order": [],
                "active-workspace-roots": "/tmp/project-open",
                "electron-workspace-root-labels": {
                    "/tmp/project-open": "Open"
                },
                "open-in-target-preferences": {
                    "perPath": {
                        "/tmp/project-open": "external"
                    }
                }
            })
            .to_string(),
        )
        .unwrap();
    create_threads_db(
        source_home.child("state_5.sqlite").path(),
        &[thread_row(
            "thread-open",
            "openai",
            "/tmp/old-project",
            0,
            "sessions/2026/05/28/rollout-2026-05-28T10-00-00-thread-open.jsonl",
        )],
    );

    let outcome =
        sync_source_history_provider(source_home.path(), HistorySyncProvider::ModexApiKey).unwrap();

    assert_eq!(outcome.changed_session_files, 1);
    assert_eq!(outcome.sqlite_provider_rows_updated, 1);
    assert_eq!(outcome.sqlite_user_event_rows_updated, 1);
    assert_eq!(outcome.sqlite_cwd_rows_updated, 1);
    assert_eq!(outcome.saved_workspace_root_count, 1);
    assert!(outcome
        .encrypted_content_warning
        .as_deref()
        .unwrap()
        .contains("invalid_encrypted_content"));
    assert_eq!(
        rollout_provider(rollout.path()).as_deref(),
        Some("modex-api-key")
    );
    assert!(fs::read_to_string(rollout.path())
        .unwrap()
        .contains(r#""encrypted_content":"ciphertext-from-openai""#));
    assert_eq!(
        fs::metadata(rollout.path()).unwrap().modified().ok(),
        Some(original_mtime)
    );
    assert_eq!(
        thread_detail(source_home.child("state_5.sqlite").path(), "thread-open"),
        Some((
            "modex-api-key".to_string(),
            "/tmp/project-open".to_string(),
            1
        ))
    );
    let global_state: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(source_home.child(".codex-global-state.json").path()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        global_state["project-order"],
        serde_json::json!(["/tmp/project-open"])
    );
    assert!(source_home
        .child(".codex-global-state.json.bak")
        .path()
        .exists());
}

#[test]
fn prepare_identity_for_launch_preserves_existing_syncs_and_repairs_history_metadata() {
    let temp = assert_fs::TempDir::new().unwrap();
    let source_home = temp.path().join("source");
    let api_home = temp.path().join(".modex/api");
    let sessions_path = source_home.join("sessions/2026/05/28");
    fs::create_dir_all(&sessions_path).unwrap();
    fs::create_dir_all(&api_home).unwrap();
    fs::write(source_home.join("config.toml"), "model = \"gpt-5.2\"\n").unwrap();
    fs::write(
        api_home.join("config.toml"),
        "[plugins.\"browser@openai-bundled\"]\nenabled = true\n",
    )
    .unwrap();
    fs::write(
        api_home.join("auth.json"),
        r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-test"}"#,
    )
    .unwrap();
    let rollout = sessions_path.join("rollout-2026-05-28T10-30-00-thread-api.jsonl");
    fs::write(
        &rollout,
        current_rollout_jsonl("openai", "thread-api", "/tmp/project-api", false),
    )
    .unwrap();
    create_threads_db(
        &source_home.join("state_5.sqlite"),
        &[thread_row(
            "thread-api",
            "openai",
            "/tmp/project-api",
            0,
            "sessions/2026/05/28/rollout-2026-05-28T10-30-00-thread-api.jsonl",
        )],
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

    let outcome = prepare_identity_for_launch(&settings, &identity).unwrap();

    assert_eq!(
        fs::read_to_string(source_home.join("auth.json")).unwrap(),
        r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-test"}"#
    );
    let config = fs::read_to_string(source_home.join("config.toml")).unwrap();
    assert!(config.contains(r#"model_provider = "modex-api-key""#));
    assert!(config.contains(r#"[plugins."browser@openai-bundled"]"#));
    assert_eq!(rollout_provider(&rollout).as_deref(), Some("modex-api-key"));
    assert_eq!(
        thread_detail(&source_home.join("state_5.sqlite"), "thread-api").map(|detail| detail.0),
        Some("modex-api-key".to_string())
    );
    assert_eq!(outcome.history_warning, None);
}

#[test]
fn prepare_identity_for_launch_returns_encrypted_history_warning() {
    let temp = assert_fs::TempDir::new().unwrap();
    let source_home = temp.path().join("source");
    let api_home = temp.path().join(".modex/api");
    let sessions_path = source_home.join("sessions/2026/05/28");
    fs::create_dir_all(&sessions_path).unwrap();
    fs::create_dir_all(&api_home).unwrap();
    fs::write(
        api_home.join("auth.json"),
        r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-test"}"#,
    )
    .unwrap();
    fs::write(
        sessions_path.join("rollout-2026-05-28T11-00-00-thread-encrypted.jsonl"),
        current_rollout_jsonl("openai", "thread-encrypted", "/tmp/project-encrypted", true),
    )
    .unwrap();

    let mut settings = AppSettings::default_for_home(temp.path().to_path_buf());
    settings.source_home = source_home;
    let identity = AppIdentity {
        name: "Gateway".to_string(),
        codex_home: api_home,
        monitor: false,
        workspace_id: None,
        auth_type: IdentityAuthType::ApiKey,
        api_base_url: Some("https://gateway.example/v1".to_string()),
    };

    let outcome = prepare_identity_for_launch(&settings, &identity).unwrap();

    assert!(outcome
        .history_warning
        .as_deref()
        .unwrap()
        .contains("invalid_encrypted_content"));
}

#[test]
fn prepare_identity_for_launch_does_not_fail_when_history_metadata_cannot_sync() {
    let temp = assert_fs::TempDir::new().unwrap();
    let source_home = temp.path().join("source");
    let api_home = temp.path().join(".modex/api");
    fs::create_dir_all(&source_home).unwrap();
    fs::create_dir_all(&api_home).unwrap();
    fs::write(source_home.join("state_5.sqlite"), "not a sqlite database").unwrap();
    fs::write(
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

    prepare_identity_for_launch(&settings, &identity).unwrap();

    assert_eq!(
        fs::read_to_string(source_home.join("auth.json")).unwrap(),
        r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-test"}"#
    );
    assert!(fs::read_to_string(source_home.join("config.toml"))
        .unwrap()
        .contains(r#"model_provider = "modex-api-key""#));
}

#[derive(Clone)]
struct ThreadRow<'a> {
    id: &'a str,
    model_provider: &'a str,
    cwd: &'a str,
    has_user_event: i64,
    rollout_path: &'a str,
}

fn thread_row<'a>(
    id: &'a str,
    model_provider: &'a str,
    cwd: &'a str,
    has_user_event: i64,
    rollout_path: &'a str,
) -> ThreadRow<'a> {
    ThreadRow {
        id,
        model_provider,
        cwd,
        has_user_event,
        rollout_path,
    }
}

fn create_threads_db(path: &Path, threads: &[ThreadRow<'_>]) {
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                cwd TEXT,
                rollout_path TEXT NOT NULL,
                model_provider TEXT,
                has_user_event INTEGER NOT NULL DEFAULT 0,
                archived INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER,
                updated_at INTEGER
            );",
        )
        .unwrap();
    for thread in threads {
        connection
            .execute(
                "INSERT INTO threads (
                    id,
                    title,
                    cwd,
                    rollout_path,
                    model_provider,
                    has_user_event,
                    archived,
                    created_at,
                    updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, 1710000000, 1710000000)",
                params![
                    thread.id,
                    format!("Title {}", thread.id),
                    thread.cwd,
                    thread.rollout_path,
                    thread.model_provider,
                    thread.has_user_event
                ],
            )
            .unwrap();
    }
}

fn thread_detail(path: &Path, thread_id: &str) -> Option<(String, String, i64)> {
    let connection = Connection::open(path).unwrap();
    connection
        .query_row(
            "SELECT model_provider, cwd, has_user_event
             FROM threads
             WHERE id = ?1",
            [thread_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .ok()
}

fn rollout_provider(path: &Path) -> Option<String> {
    let payload = first_rollout_json(path);
    payload
        .get("payload")
        .and_then(|value| value.get("model_provider"))
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string)
}

fn first_rollout_json(path: &Path) -> serde_json::Value {
    let first_line = fs::read_to_string(path)
        .unwrap()
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();
    serde_json::from_str(&first_line).unwrap()
}

fn current_rollout_jsonl(
    provider: &str,
    thread_id: &str,
    cwd: &str,
    include_encrypted_content: bool,
) -> String {
    let mut lines = vec![
        serde_json::json!({
            "timestamp": "2026-05-28T02:00:00.000Z",
            "type": "session_meta",
            "payload": {
                "id": thread_id,
                "timestamp": "2026-05-28T02:00:00.000Z",
                "cwd": cwd,
                "source": "vscode",
                "model_provider": provider
            }
        })
        .to_string(),
        serde_json::json!({
            "timestamp": "2026-05-28T02:01:00.000Z",
            "type": "event_msg",
            "payload": {
                "type": "user_message"
            }
        })
        .to_string(),
    ];
    if include_encrypted_content {
        lines.push(
            serde_json::json!({
                "timestamp": "2026-05-28T02:02:00.000Z",
                "type": "response_item",
                "payload": {
                    "encrypted_content": "ciphertext-from-openai"
                }
            })
            .to_string(),
        );
    }
    format!("{}\n", lines.join("\n"))
}

fn set_mtime(path: &Path, mtime: std::time::SystemTime) {
    let file = fs::OpenOptions::new().write(true).open(path).unwrap();
    file.set_times(fs::FileTimes::new().set_modified(mtime))
        .unwrap();
}

fn identity(auth_type: IdentityAuthType, api_base_url: Option<&str>) -> AppIdentity {
    AppIdentity {
        name: "Identity".to_string(),
        codex_home: "/tmp/modex-test/identity".into(),
        monitor: false,
        workspace_id: None,
        auth_type,
        api_base_url: api_base_url.map(ToString::to_string),
    }
}

use std::fs;
use std::path::Path;

use assert_fs::prelude::*;
use modex_lib::core::sync::{sync_source_history_provider, HistorySyncProvider};
use rusqlite::{params, Connection};

#[test]
fn sync_source_history_provider_retags_threads_and_rollouts_bidirectionally() {
    let temp = assert_fs::TempDir::new().unwrap();
    let source_home = temp.child("source");
    source_home.create_dir_all().unwrap();
    let state_path = source_home.child("state_5.sqlite");
    let sessions_path = source_home.child("sessions");
    let archived_sessions_path = source_home.child("archived_sessions");
    sessions_path.create_dir_all().unwrap();
    archived_sessions_path.create_dir_all().unwrap();

    let primary_rollout = sessions_path.child("project-a.jsonl");
    primary_rollout
        .write_str(&rollout_jsonl(
            "openai",
            "thread-open",
            false,
            "/tmp/project-a",
        ))
        .unwrap();
    let archived_rollout = archived_sessions_path.child("project-b.jsonl");
    archived_rollout
        .write_str(&rollout_jsonl(
            "openai",
            "thread-archived",
            true,
            "/tmp/project-b",
        ))
        .unwrap();

    create_threads_db(
        state_path.path(),
        &[
            thread_row("thread-open", "openai", "sessions/project-a.jsonl", false),
            thread_row(
                "thread-archived",
                "openai",
                "archived_sessions/project-b.jsonl",
                true,
            ),
        ],
    );

    sync_source_history_provider(source_home.path(), HistorySyncProvider::ModexApiKey).unwrap();

    assert_eq!(
        thread_rows(state_path.path()),
        vec![
            (
                "thread-archived".to_string(),
                "modex-api-key".to_string(),
                1,
                "archived_sessions/project-b.jsonl".to_string()
            ),
            (
                "thread-open".to_string(),
                "modex-api-key".to_string(),
                0,
                "sessions/project-a.jsonl".to_string()
            ),
        ]
    );
    assert_eq!(
        rollout_provider(primary_rollout.path()).as_deref(),
        Some("modex-api-key")
    );
    assert_eq!(
        rollout_provider(archived_rollout.path()).as_deref(),
        Some("modex-api-key")
    );

    sync_source_history_provider(source_home.path(), HistorySyncProvider::OpenAi).unwrap();

    assert_eq!(
        thread_rows(state_path.path()),
        vec![
            (
                "thread-archived".to_string(),
                "openai".to_string(),
                1,
                "archived_sessions/project-b.jsonl".to_string()
            ),
            (
                "thread-open".to_string(),
                "openai".to_string(),
                0,
                "sessions/project-a.jsonl".to_string()
            ),
        ]
    );
    assert_eq!(
        rollout_provider(primary_rollout.path()).as_deref(),
        Some("openai")
    );
    assert_eq!(
        rollout_provider(archived_rollout.path()).as_deref(),
        Some("openai")
    );
}

#[derive(Clone)]
struct ThreadRow<'a> {
    id: &'a str,
    model_provider: &'a str,
    rollout_path: &'a str,
    archived: bool,
}

fn thread_row<'a>(
    id: &'a str,
    model_provider: &'a str,
    rollout_path: &'a str,
    archived: bool,
) -> ThreadRow<'a> {
    ThreadRow {
        id,
        model_provider,
        rollout_path,
        archived,
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
                    archived,
                    created_at,
                    updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1710000000, 1710000000)",
                params![
                    thread.id,
                    format!("Title {}", thread.id),
                    format!("/tmp/{}", thread.id),
                    thread.rollout_path,
                    thread.model_provider,
                    if thread.archived { 1 } else { 0 }
                ],
            )
            .unwrap();
    }
}

fn thread_rows(path: &Path) -> Vec<(String, String, i64, String)> {
    let connection = Connection::open(path).unwrap();
    let mut statement = connection
        .prepare(
            "SELECT id, model_provider, archived, rollout_path
             FROM threads
             ORDER BY id",
        )
        .unwrap();
    statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn rollout_provider(path: &Path) -> Option<String> {
    let first_line = fs::read_to_string(path)
        .unwrap()
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();
    let payload: serde_json::Value = serde_json::from_str(&first_line).unwrap();
    payload
        .get("session_meta")
        .and_then(|value| value.get("payload"))
        .and_then(|value| value.get("model_provider"))
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string)
}

fn rollout_jsonl(provider: &str, thread_id: &str, archived: bool, cwd: &str) -> String {
    format!(
        "{}\n{}\n",
        serde_json::json!({
            "session_meta": {
                "id": thread_id,
                "payload": {
                    "model_provider": provider,
                    "cwd": cwd,
                    "archived": archived
                }
            }
        }),
        serde_json::json!({
            "event": "message",
            "thread_id": thread_id
        })
    )
}

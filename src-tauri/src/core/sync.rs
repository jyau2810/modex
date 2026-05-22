use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use rusqlite::types::Value as SqlValue;
use rusqlite::{params, params_from_iter, Connection, Transaction};
use serde::Deserialize;
use serde_json::Value;

use super::app_config::{AppIdentity, IdentityAuthType};
use super::{ModexError, ModexResult};

const STATE_DB_NAME: &str = "state_5.sqlite";
const SESSION_INDEX_NAME: &str = "session_index.jsonl";
const ACTIVE_SESSIONS_DIR: &str = "sessions";
const ARCHIVED_SESSIONS_DIR: &str = "archived_sessions";
const OPENAI_PROVIDER_ID: &str = "openai";
const MODEX_API_KEY_PROVIDER_ID: &str = "modex-api-key";
const DEFAULT_THREAD_SOURCE: &str = "vscode";
const DEFAULT_SANDBOX_POLICY: &str = r#"{"type":"danger-full-access"}"#;
const DEFAULT_APPROVAL_MODE: &str = "never";
const DEFAULT_MEMORY_MODE: &str = "enabled";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistorySyncProvider {
    OpenAi,
    ModexApiKey,
}

impl HistorySyncProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAi => OPENAI_PROVIDER_ID,
            Self::ModexApiKey => MODEX_API_KEY_PROVIDER_ID,
        }
    }
}

impl From<&IdentityAuthType> for HistorySyncProvider {
    fn from(value: &IdentityAuthType) -> Self {
        match value {
            IdentityAuthType::ChatGpt => Self::OpenAi,
            IdentityAuthType::ApiKey => Self::ModexApiKey,
        }
    }
}

impl From<IdentityAuthType> for HistorySyncProvider {
    fn from(value: IdentityAuthType) -> Self {
        Self::from(&value)
    }
}

pub fn history_sync_provider_for_identity(identity: &AppIdentity) -> HistorySyncProvider {
    match identity.auth_type {
        IdentityAuthType::ChatGpt => HistorySyncProvider::OpenAi,
        IdentityAuthType::ApiKey => identity
            .api_base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|_| HistorySyncProvider::ModexApiKey)
            .unwrap_or(HistorySyncProvider::OpenAi),
    }
}

pub fn sync_identity_auth(source_home: &Path, identity_home: &Path) -> ModexResult<PathBuf> {
    fs::create_dir_all(source_home)?;
    let source_auth = source_home.join("auth.json");
    let identity_auth = identity_home.join("auth.json");
    if !identity_auth.exists() {
        return Err(ModexError::from(format!(
            "账号缺少登录凭据：{}",
            identity_auth.display()
        )));
    }
    let temporary = source_auth.with_file_name("auth.json.modex-tmp");
    fs::copy(&identity_auth, &temporary)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600));
    }
    fs::rename(&temporary, &source_auth)?;
    Ok(source_auth)
}

pub fn sync_source_history_provider(
    source_home: &Path,
    provider: HistorySyncProvider,
) -> ModexResult<()> {
    let state_path = source_home.join(STATE_DB_NAME);
    if !state_path.exists() {
        return Ok(());
    }

    let session_index = load_session_index(source_home)?;
    let rollout_threads = collect_rollout_threads(source_home, &session_index)?;

    let mut connection = Connection::open(&state_path)?;
    let transaction = connection.transaction()?;
    let thread_columns = load_thread_columns(&transaction)?;

    retag_provider_view(&transaction, provider)?;

    let mut existing_ids = load_existing_thread_ids(&transaction)?;
    for rollout_thread in rollout_threads {
        if !existing_ids.insert(rollout_thread.id.clone()) {
            continue;
        }
        insert_thread_row(&transaction, &thread_columns, &rollout_thread, provider)?;
    }

    transaction.commit()?;
    Ok(())
}

fn retag_provider_view(
    transaction: &Transaction<'_>,
    provider: HistorySyncProvider,
) -> ModexResult<()> {
    transaction.execute(
        "UPDATE threads
         SET model_provider = ?1
         WHERE model_provider IS NULL
            OR TRIM(model_provider) = ''
            OR model_provider = ?2
            OR model_provider = ?3",
        params![
            provider.as_str(),
            OPENAI_PROVIDER_ID,
            MODEX_API_KEY_PROVIDER_ID
        ],
    )?;
    Ok(())
}

fn load_existing_thread_ids(transaction: &Transaction<'_>) -> ModexResult<HashSet<String>> {
    let mut statement = transaction.prepare("SELECT id FROM threads")?;
    let ids = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<HashSet<_>, _>>()
        .map_err(ModexError::from)?;
    Ok(ids)
}

fn load_thread_columns(transaction: &Transaction<'_>) -> ModexResult<Vec<ThreadColumn>> {
    let mut statement = transaction.prepare("PRAGMA table_info(threads)")?;
    let columns = statement
        .query_map([], |row| {
            Ok(ThreadColumn {
                name: row.get(1)?,
                declared_type: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                not_null: row.get::<_, i64>(3)? != 0,
                default_value: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    if columns.is_empty() {
        return Err(ModexError::from("state_5.sqlite 缺少 threads 表结构"));
    }
    Ok(columns)
}

fn insert_thread_row(
    transaction: &Transaction<'_>,
    thread_columns: &[ThreadColumn],
    rollout_thread: &RolloutThread,
    provider: HistorySyncProvider,
) -> ModexResult<()> {
    let created_at = resolve_created_at(transaction, rollout_thread)?;
    let updated_at = resolve_updated_at(transaction, rollout_thread, created_at)?;
    let rollout_path = rollout_thread.rollout_path.display().to_string();
    let title = rollout_thread
        .title
        .clone()
        .unwrap_or_else(|| rollout_thread.id.clone());
    let cwd = rollout_thread.cwd.clone().unwrap_or_default();
    let source = rollout_thread
        .source
        .clone()
        .unwrap_or_else(|| DEFAULT_THREAD_SOURCE.to_string());
    let cli_version = rollout_thread.cli_version.clone().unwrap_or_default();

    let mut column_names = Vec::new();
    let mut values = Vec::new();
    for column in thread_columns {
        let value = match column.name.as_str() {
            "id" => Some(SqlValue::Text(rollout_thread.id.clone())),
            "rollout_path" => Some(SqlValue::Text(rollout_path.clone())),
            "created_at" => Some(SqlValue::Integer(created_at)),
            "updated_at" => Some(SqlValue::Integer(updated_at)),
            "created_at_ms" => Some(SqlValue::Integer(created_at.saturating_mul(1000))),
            "updated_at_ms" => Some(SqlValue::Integer(updated_at.saturating_mul(1000))),
            "source" => Some(SqlValue::Text(source.clone())),
            "model_provider" => Some(SqlValue::Text(provider.as_str().to_string())),
            "cwd" => Some(SqlValue::Text(cwd.clone())),
            "title" => Some(SqlValue::Text(title.clone())),
            "sandbox_policy" => Some(SqlValue::Text(DEFAULT_SANDBOX_POLICY.to_string())),
            "approval_mode" => Some(SqlValue::Text(DEFAULT_APPROVAL_MODE.to_string())),
            "archived" => Some(SqlValue::Integer(i64::from(rollout_thread.archived))),
            "archived_at" if rollout_thread.archived => Some(SqlValue::Integer(updated_at)),
            "cli_version" => Some(SqlValue::Text(cli_version.clone())),
            "first_user_message" => Some(SqlValue::Text(String::new())),
            "memory_mode" => Some(SqlValue::Text(DEFAULT_MEMORY_MODE.to_string())),
            "preview" => Some(SqlValue::Text(String::new())),
            _ => None,
        };

        if let Some(value) = value {
            column_names.push(column.name.clone());
            values.push(value);
            continue;
        }

        if column.not_null && column.default_value.is_none() {
            column_names.push(column.name.clone());
            values.push(fallback_value_for_column(column));
        }
    }

    let placeholders = (1..=column_names.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>();
    let sql = format!(
        "INSERT INTO threads ({}) VALUES ({})",
        column_names.join(", "),
        placeholders.join(", ")
    );
    transaction.execute(&sql, params_from_iter(values))?;
    Ok(())
}

fn fallback_value_for_column(column: &ThreadColumn) -> SqlValue {
    let declared_type = column.declared_type.to_ascii_uppercase();
    if declared_type.contains("INT") || declared_type.contains("BOOL") {
        SqlValue::Integer(0)
    } else {
        SqlValue::Text(String::new())
    }
}

fn resolve_created_at(
    transaction: &Transaction<'_>,
    rollout_thread: &RolloutThread,
) -> ModexResult<i64> {
    if let Some(timestamp) =
        parse_timestamp_with_sqlite(transaction, rollout_thread.created_at.as_deref())?
    {
        return Ok(timestamp);
    }
    if let Some(timestamp) =
        parse_timestamp_with_sqlite(transaction, rollout_thread.updated_at.as_deref())?
    {
        return Ok(timestamp);
    }
    Ok(file_timestamp_seconds(&rollout_thread.rollout_path).unwrap_or_default())
}

fn resolve_updated_at(
    transaction: &Transaction<'_>,
    rollout_thread: &RolloutThread,
    created_at: i64,
) -> ModexResult<i64> {
    if let Some(timestamp) =
        parse_timestamp_with_sqlite(transaction, rollout_thread.updated_at.as_deref())?
    {
        return Ok(timestamp);
    }
    if let Some(timestamp) =
        parse_timestamp_with_sqlite(transaction, rollout_thread.created_at.as_deref())?
    {
        return Ok(timestamp);
    }
    Ok(file_timestamp_seconds(&rollout_thread.rollout_path).unwrap_or(created_at))
}

fn parse_timestamp_with_sqlite(
    transaction: &Transaction<'_>,
    raw: Option<&str>,
) -> ModexResult<Option<i64>> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    transaction
        .query_row("SELECT unixepoch(?1)", [raw], |row| {
            row.get::<_, Option<i64>>(0)
        })
        .map_err(Into::into)
}

fn file_timestamp_seconds(path: &Path) -> Option<i64> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    let since_epoch = modified.duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(since_epoch.as_secs()).ok()
}

fn load_session_index(source_home: &Path) -> ModexResult<HashMap<String, SessionIndexEntry>> {
    let session_index_path = source_home.join(SESSION_INDEX_NAME);
    if !session_index_path.exists() {
        return Ok(HashMap::new());
    }

    let mut entries = HashMap::new();
    for line in fs::read_to_string(session_index_path)?.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(raw_entry) = serde_json::from_str::<RawSessionIndexEntry>(trimmed) else {
            continue;
        };
        let Some(id) = raw_entry
            .id
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        entries.insert(
            id,
            SessionIndexEntry {
                title: raw_entry
                    .thread_name
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty()),
                updated_at: raw_entry
                    .updated_at
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty()),
            },
        );
    }

    Ok(entries)
}

fn collect_rollout_threads(
    source_home: &Path,
    session_index: &HashMap<String, SessionIndexEntry>,
) -> ModexResult<Vec<RolloutThread>> {
    let mut rollout_paths = Vec::new();
    for directory_name in [ACTIVE_SESSIONS_DIR, ARCHIVED_SESSIONS_DIR] {
        let directory = source_home.join(directory_name);
        if !directory.is_dir() {
            continue;
        }
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            if path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
                continue;
            }
            rollout_paths.push(path);
        }
    }
    rollout_paths.sort();

    let mut by_thread_id = HashMap::new();
    for rollout_path in rollout_paths {
        if let Some(rollout_thread) =
            parse_rollout_thread(source_home, &rollout_path, session_index)?
        {
            by_thread_id
                .entry(rollout_thread.id.clone())
                .and_modify(|existing| merge_rollout_threads(existing, &rollout_thread))
                .or_insert(rollout_thread);
        }
    }

    let mut rollout_threads = by_thread_id.into_values().collect::<Vec<_>>();
    rollout_threads.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(rollout_threads)
}

fn parse_rollout_thread(
    source_home: &Path,
    rollout_path: &Path,
    session_index: &HashMap<String, SessionIndexEntry>,
) -> ModexResult<Option<RolloutThread>> {
    let first_line = fs::read_to_string(rollout_path)?
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();
    let first_line_json = serde_json::from_str::<Value>(&first_line).ok();
    let payload = first_line_json.as_ref().and_then(extract_rollout_payload);

    let id = payload
        .and_then(|value| value.get("id"))
        .and_then(Value::as_str)
        .or_else(|| {
            first_line_json
                .as_ref()
                .and_then(|value| value.get("session_meta"))
                .and_then(|value| value.get("id"))
                .and_then(Value::as_str)
        })
        .or_else(|| {
            first_line_json
                .as_ref()
                .and_then(|value| value.get("payload"))
                .and_then(|value| value.get("thread_id"))
                .and_then(Value::as_str)
        })
        .or_else(|| parse_thread_id_from_filename(rollout_path))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let Some(id) = id else {
        return Ok(None);
    };

    let index_entry = session_index.get(&id);
    let archived = payload
        .and_then(|value| value.get("archived"))
        .and_then(Value::as_bool)
        .unwrap_or_else(|| is_archived_rollout(source_home, rollout_path));

    Ok(Some(RolloutThread {
        id,
        title: index_entry.and_then(|entry| entry.title.clone()),
        cwd: payload
            .and_then(|value| value.get("cwd"))
            .and_then(Value::as_str)
            .map(ToString::to_string),
        rollout_path: rollout_path.to_path_buf(),
        archived,
        source: payload
            .and_then(|value| value.get("source"))
            .and_then(Value::as_str)
            .map(ToString::to_string),
        cli_version: payload
            .and_then(|value| value.get("cli_version"))
            .and_then(Value::as_str)
            .map(ToString::to_string),
        created_at: payload
            .and_then(|value| value.get("timestamp"))
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .or_else(|| {
                first_line_json
                    .as_ref()
                    .and_then(|value| value.get("timestamp"))
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            }),
        updated_at: index_entry
            .and_then(|entry| entry.updated_at.clone())
            .or_else(|| {
                first_line_json
                    .as_ref()
                    .and_then(|value| value.get("timestamp"))
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            }),
    }))
}

fn merge_rollout_threads(existing: &mut RolloutThread, next: &RolloutThread) {
    if existing.title.is_none() {
        existing.title = next.title.clone();
    }
    if existing.cwd.is_none() {
        existing.cwd = next.cwd.clone();
    }
    if existing.source.is_none() {
        existing.source = next.source.clone();
    }
    if existing.cli_version.is_none() {
        existing.cli_version = next.cli_version.clone();
    }
    if existing.created_at.is_none() {
        existing.created_at = next.created_at.clone();
    }
    if existing.updated_at.is_none() {
        existing.updated_at = next.updated_at.clone();
    }
    if !existing.archived {
        existing.archived = next.archived;
    }
}

fn extract_rollout_payload(value: &Value) -> Option<&Value> {
    value
        .get("session_meta")
        .and_then(|inner| inner.get("payload"))
        .or_else(|| {
            value
                .get("type")
                .and_then(Value::as_str)
                .filter(|kind| *kind == "session_meta")
                .and_then(|_| value.get("payload"))
        })
}

fn is_archived_rollout(source_home: &Path, rollout_path: &Path) -> bool {
    rollout_path
        .strip_prefix(source_home)
        .ok()
        .and_then(|relative| relative.components().next())
        .is_some_and(|component| component.as_os_str() == ARCHIVED_SESSIONS_DIR)
}

fn parse_thread_id_from_filename(path: &Path) -> Option<&str> {
    let stem = path.file_stem()?.to_str()?;
    let candidate = stem.get(stem.len().checked_sub(36)?..)?;
    candidate
        .chars()
        .all(|value| value.is_ascii_hexdigit() || value == '-')
        .then_some(candidate)
}

#[derive(Debug)]
struct ThreadColumn {
    name: String,
    declared_type: String,
    not_null: bool,
    default_value: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct SessionIndexEntry {
    title: Option<String>,
    updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawSessionIndexEntry {
    id: Option<String>,
    thread_name: Option<String>,
    updated_at: Option<String>,
}

#[derive(Clone, Debug)]
struct RolloutThread {
    id: String,
    title: Option<String>,
    cwd: Option<String>,
    rollout_path: PathBuf,
    archived: bool,
    source: Option<String>,
    cli_version: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
}

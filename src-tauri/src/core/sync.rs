use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::{Map, Value};

use super::app_config::{AppIdentity, IdentityAuthType};
use super::{ModexError, ModexResult};

const STATE_DB_NAME: &str = "state_5.sqlite";
const GLOBAL_STATE_FILE_NAME: &str = ".codex-global-state.json";
const GLOBAL_STATE_BACKUP_FILE_NAME: &str = ".codex-global-state.json.bak";
const ACTIVE_SESSIONS_DIR: &str = "sessions";
const ARCHIVED_SESSIONS_DIR: &str = "archived_sessions";
const OPENAI_PROVIDER_ID: &str = "openai";
const MODEX_API_KEY_PROVIDER_ID: &str = "modex-api-key";

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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HistorySyncOutcome {
    pub changed_session_files: usize,
    pub skipped_session_files: usize,
    pub sanitized_encrypted_content_fields: usize,
    pub sqlite_present: bool,
    pub sqlite_provider_rows_updated: usize,
    pub sqlite_user_event_rows_updated: usize,
    pub sqlite_cwd_rows_updated: usize,
    pub updated_workspace_roots: usize,
    pub saved_workspace_root_count: usize,
    pub encrypted_content_warning: Option<String>,
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
) -> ModexResult<HistorySyncOutcome> {
    let target_provider = provider.as_str();
    let scan = collect_session_changes(source_home, target_provider)?;
    let mut outcome = HistorySyncOutcome::default();

    let state_path = source_home.join(STATE_DB_NAME);
    if !state_path.exists() {
        let rollout_outcome = apply_rollout_rewrites(&scan.changes)?;
        outcome.changed_session_files = rollout_outcome.applied.len();
        outcome.skipped_session_files = rollout_outcome.skipped;
        outcome.sanitized_encrypted_content_fields =
            rollout_outcome.sanitized_encrypted_content_fields;
        outcome.encrypted_content_warning = build_encrypted_content_warning(
            &scan.encrypted_content_counts,
            target_provider,
            outcome.sanitized_encrypted_content_fields,
        );
        let workspace_outcome = sync_workspace_roots(source_home, &scan.thread_cwd_by_id)?;
        outcome.updated_workspace_roots = workspace_outcome.updated_workspace_roots;
        outcome.saved_workspace_root_count = workspace_outcome.saved_workspace_root_count;
        return Ok(outcome);
    }

    outcome.sqlite_present = true;
    let mut applied_rollouts = Vec::new();
    let result = (|| {
        let mut connection = Connection::open(&state_path).map_err(sqlite_error)?;
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(sqlite_error)?;
        let transaction = connection.transaction().map_err(sqlite_error)?;
        let sqlite_outcome = update_sqlite_threads(
            &transaction,
            target_provider,
            &scan.user_event_thread_ids,
            &scan.thread_cwd_by_id,
        )?;
        let rollout_outcome = apply_rollout_rewrites(&scan.changes)?;
        applied_rollouts = rollout_outcome.applied.clone();
        let workspace_outcome = sync_workspace_roots(source_home, &scan.thread_cwd_by_id)?;
        transaction.commit().map_err(sqlite_error)?;
        Ok((sqlite_outcome, rollout_outcome, workspace_outcome))
    })();

    match result {
        Ok((sqlite_outcome, rollout_outcome, workspace_outcome)) => {
            outcome.changed_session_files = applied_rollouts.len();
            outcome.skipped_session_files = rollout_outcome.skipped;
            outcome.sanitized_encrypted_content_fields =
                rollout_outcome.sanitized_encrypted_content_fields;
            outcome.encrypted_content_warning = build_encrypted_content_warning(
                &scan.encrypted_content_counts,
                target_provider,
                outcome.sanitized_encrypted_content_fields,
            );
            outcome.sqlite_provider_rows_updated = sqlite_outcome.provider_rows_updated;
            outcome.sqlite_user_event_rows_updated = sqlite_outcome.user_event_rows_updated;
            outcome.sqlite_cwd_rows_updated = sqlite_outcome.cwd_rows_updated;
            outcome.updated_workspace_roots = workspace_outcome.updated_workspace_roots;
            outcome.saved_workspace_root_count = workspace_outcome.saved_workspace_root_count;
            Ok(outcome)
        }
        Err(error) => {
            restore_rollout_first_lines(&applied_rollouts)?;
            Err(error)
        }
    }
}

fn update_sqlite_threads(
    transaction: &Transaction<'_>,
    target_provider: &str,
    user_event_thread_ids: &BTreeSet<String>,
    thread_cwd_by_id: &BTreeMap<String, String>,
) -> ModexResult<SqliteUpdateOutcome> {
    if !table_exists(transaction, "threads")? {
        return Ok(SqliteUpdateOutcome::default());
    }

    let has_model_provider = table_has_column(transaction, "threads", "model_provider")?;
    let has_user_event = table_has_column(transaction, "threads", "has_user_event")?;
    let has_cwd = table_has_column(transaction, "threads", "cwd")?;

    let mut outcome = SqliteUpdateOutcome::default();
    if has_model_provider {
        let rows = transaction
            .execute(
                "UPDATE threads
                 SET model_provider = ?1
                 WHERE COALESCE(model_provider, '') <> ?1",
                [target_provider],
            )
            .map_err(sqlite_error)?;
        outcome.provider_rows_updated = rows;
    }

    if has_user_event {
        let mut statement = transaction
            .prepare(
                "UPDATE threads
                 SET has_user_event = 1
                 WHERE id = ?1 AND COALESCE(has_user_event, 0) <> 1",
            )
            .map_err(sqlite_error)?;
        for thread_id in user_event_thread_ids {
            outcome.user_event_rows_updated +=
                statement.execute([thread_id]).map_err(sqlite_error)?;
        }
    }

    if has_cwd {
        let mut statement = transaction
            .prepare(
                "UPDATE threads
                 SET cwd = ?1
                 WHERE id = ?2 AND COALESCE(cwd, '') <> ?1",
            )
            .map_err(sqlite_error)?;
        for (thread_id, cwd) in thread_cwd_by_id {
            if thread_id.trim().is_empty() || cwd.trim().is_empty() {
                continue;
            }
            outcome.cwd_rows_updated += statement
                .execute(params![cwd, thread_id])
                .map_err(sqlite_error)?;
        }
    }

    Ok(outcome)
}

fn table_exists(transaction: &Transaction<'_>, table_name: &str) -> ModexResult<bool> {
    let exists = transaction
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table_name],
            |_| Ok(()),
        )
        .optional()
        .map_err(sqlite_error)?
        .is_some();
    Ok(exists)
}

fn table_has_column(
    transaction: &Transaction<'_>,
    table_name: &str,
    column_name: &str,
) -> ModexResult<bool> {
    let escaped_table_name = table_name.replace('"', "\"\"");
    let mut statement = transaction
        .prepare(&format!("PRAGMA table_info(\"{escaped_table_name}\")"))
        .map_err(sqlite_error)?;
    let mut rows = statement.query([]).map_err(sqlite_error)?;
    while let Some(row) = rows.next().map_err(sqlite_error)? {
        let name: String = row.get(1).map_err(sqlite_error)?;
        if name == column_name {
            return Ok(true);
        }
    }
    Ok(false)
}

fn collect_session_changes(source_home: &Path, target_provider: &str) -> ModexResult<SessionScan> {
    let mut scan = SessionScan::default();
    for directory_name in [ACTIVE_SESSIONS_DIR, ARCHIVED_SESSIONS_DIR] {
        let directory = source_home.join(directory_name);
        collect_session_changes_in_directory(&directory, target_provider, &mut scan)?;
    }
    Ok(scan)
}

fn collect_session_changes_in_directory(
    directory: &Path,
    target_provider: &str,
    scan: &mut SessionScan,
) -> ModexResult<()> {
    if !directory.is_dir() {
        return Ok(());
    }

    let mut entries = fs::read_dir(directory)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(ModexError::from)?;
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_session_changes_in_directory(&path, target_provider, scan)?;
            continue;
        }
        if !is_rollout_file(&path) {
            continue;
        }
        collect_rollout_change(&path, target_provider, scan)?;
    }

    Ok(())
}

fn collect_rollout_change(
    path: &Path,
    target_provider: &str,
    scan: &mut SessionScan,
) -> ModexResult<()> {
    let contents = fs::read_to_string(path)?;
    let snapshot = FileSnapshot::capture(path)?;
    let Some(record) = FirstLineRecord::from_contents(&contents) else {
        return Ok(());
    };
    let Ok(mut first_line_json) = serde_json::from_str::<Value>(&record.first_line) else {
        return Ok(());
    };
    let Some(payload) = extract_session_meta_payload_mut(&mut first_line_json) else {
        return Ok(());
    };
    let Some(payload_object) = payload.as_object_mut() else {
        return Ok(());
    };

    let current_provider = payload_object
        .get("model_provider")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("(missing)")
        .to_string();
    let thread_id = payload_object
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);

    if contents.contains("encrypted_content") {
        *scan
            .encrypted_content_counts
            .entry(current_provider.clone())
            .or_default() += 1;
    }
    if let Some(thread_id) = &thread_id {
        if rollout_has_user_event(&contents) {
            scan.user_event_thread_ids.insert(thread_id.clone());
        }
        if let Some(cwd) = payload_object
            .get("cwd")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            scan.thread_cwd_by_id
                .insert(thread_id.clone(), cwd.to_string());
        }
    }

    if current_provider == target_provider {
        return Ok(());
    }

    payload_object.insert(
        "model_provider".to_string(),
        Value::String(target_provider.to_string()),
    );
    let updated_first_line = serde_json::to_string(&first_line_json)?;
    let remove_encrypted_content = contents.contains("encrypted_content");
    scan.changes.push(RolloutChange {
        path: path.to_path_buf(),
        original_first_line: record.first_line,
        original_separator: record.separator,
        snapshot,
        updated_first_line,
        remove_encrypted_content,
        original_contents: remove_encrypted_content.then(|| Arc::<str>::from(contents.as_str())),
    });
    Ok(())
}

fn apply_rollout_rewrites(changes: &[RolloutChange]) -> ModexResult<RolloutRewriteOutcome> {
    let mut applied = Vec::new();
    let mut skipped = 0;
    let mut sanitized_encrypted_content_fields = 0;
    for change in changes {
        match write_rollout_if_unchanged(change, &change.updated_first_line) {
            Ok(Some(write_outcome)) => {
                sanitized_encrypted_content_fields +=
                    write_outcome.sanitized_encrypted_content_fields;
                applied.push(change.clone());
            }
            Ok(None) => skipped += 1,
            Err(error) => {
                restore_rollout_first_lines(&applied)?;
                return Err(error);
            }
        }
    }
    Ok(RolloutRewriteOutcome {
        applied,
        skipped,
        sanitized_encrypted_content_fields,
    })
}

fn restore_rollout_first_lines(changes: &[RolloutChange]) -> ModexResult<()> {
    for change in changes.iter().rev() {
        restore_rollout_first_line(change)?;
    }
    Ok(())
}

fn write_rollout_if_unchanged(
    change: &RolloutChange,
    first_line: &str,
) -> ModexResult<Option<RolloutWriteOutcome>> {
    if !change.snapshot.matches_path(&change.path)? {
        return Ok(None);
    }
    let current = fs::read_to_string(&change.path)?;
    let Some(record) = FirstLineRecord::from_contents(&current) else {
        return Ok(None);
    };
    if record.first_line != change.original_first_line
        || record.separator != change.original_separator
    {
        return Ok(None);
    }
    let (next, sanitized_encrypted_content_fields) = build_rollout_contents(
        first_line,
        &record.separator,
        &record.rest,
        change.remove_encrypted_content,
    )?;
    if !change.snapshot.matches_path(&change.path)? {
        return Ok(None);
    }
    write_rollout_contents(change, &next)?;
    restore_rollout_mtime(&change.path, change.snapshot.modified);
    Ok(Some(RolloutWriteOutcome {
        sanitized_encrypted_content_fields,
    }))
}

fn restore_rollout_first_line(change: &RolloutChange) -> ModexResult<()> {
    if let Some(original_contents) = &change.original_contents {
        write_rollout_contents(change, original_contents)?;
        restore_rollout_mtime(&change.path, change.snapshot.modified);
        return Ok(());
    }
    let current = fs::read_to_string(&change.path)?;
    let Some(record) = FirstLineRecord::from_contents(&current) else {
        return Ok(());
    };
    let separator = if record.separator.is_empty() {
        &change.original_separator
    } else {
        &record.separator
    };
    let (next, _) =
        build_rollout_contents(&change.original_first_line, separator, &record.rest, false)?;
    write_rollout_contents(change, &next)?;
    restore_rollout_mtime(&change.path, change.snapshot.modified);
    Ok(())
}

fn build_rollout_contents(
    first_line: &str,
    separator: &str,
    rest: &str,
    remove_encrypted_content: bool,
) -> ModexResult<(String, usize)> {
    let mut next = String::new();
    next.push_str(first_line);
    next.push_str(separator);
    if remove_encrypted_content {
        let (sanitized_rest, sanitized_encrypted_content_fields) =
            remove_encrypted_content_from_jsonl(rest)?;
        next.push_str(&sanitized_rest);
        Ok((next, sanitized_encrypted_content_fields))
    } else {
        next.push_str(rest);
        Ok((next, 0))
    }
}

fn remove_encrypted_content_from_jsonl(contents: &str) -> ModexResult<(String, usize)> {
    let mut sanitized = String::with_capacity(contents.len());
    let mut removed_fields = 0;
    for line in contents.split_inclusive('\n') {
        let (body, ending) = split_line_ending(line);
        if !body.contains("encrypted_content") {
            sanitized.push_str(line);
            continue;
        }
        let Ok(mut value) = serde_json::from_str::<Value>(body) else {
            sanitized.push_str(line);
            continue;
        };
        let removed = remove_encrypted_content_fields(&mut value);
        if removed == 0 {
            sanitized.push_str(line);
            continue;
        }
        sanitized.push_str(&serde_json::to_string(&value)?);
        sanitized.push_str(ending);
        removed_fields += removed;
    }
    Ok((sanitized, removed_fields))
}

fn split_line_ending(line: &str) -> (&str, &str) {
    if let Some(body) = line.strip_suffix("\r\n") {
        (body, "\r\n")
    } else if let Some(body) = line.strip_suffix('\n') {
        (body, "\n")
    } else {
        (line, "")
    }
}

fn remove_encrypted_content_fields(value: &mut Value) -> usize {
    match value {
        Value::Object(object) => {
            let mut removed = usize::from(object.remove("encrypted_content").is_some());
            for value in object.values_mut() {
                removed += remove_encrypted_content_fields(value);
            }
            removed
        }
        Value::Array(values) => values.iter_mut().map(remove_encrypted_content_fields).sum(),
        _ => 0,
    }
}

fn write_rollout_contents(change: &RolloutChange, contents: &str) -> ModexResult<()> {
    let temporary = change.path.with_file_name(format!(
        "{}.modex-provider-sync",
        change
            .path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("rollout")
    ));
    fs::write(&temporary, contents)?;
    if let Ok(metadata) = fs::metadata(&change.path) {
        let _ = fs::set_permissions(&temporary, metadata.permissions());
    }
    fs::rename(&temporary, &change.path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        ModexError::from(error)
    })?;
    Ok(())
}

fn restore_rollout_mtime(path: &Path, modified: Option<SystemTime>) {
    let Some(modified) = modified else {
        return;
    };
    let Ok(file) = fs::OpenOptions::new().write(true).open(path) else {
        return;
    };
    let _ = file.set_times(fs::FileTimes::new().set_modified(modified));
}

fn sync_workspace_roots(
    source_home: &Path,
    thread_cwd_by_id: &BTreeMap<String, String>,
) -> ModexResult<WorkspaceRootOutcome> {
    let state_path = source_home.join(GLOBAL_STATE_FILE_NAME);
    let original_text = match fs::read_to_string(&state_path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(WorkspaceRootOutcome::default());
        }
        Err(error) => return Err(error.into()),
    };
    let mut state = serde_json::from_str::<Value>(&original_text)?;
    let Some(state_object) = state.as_object_mut() else {
        return Ok(WorkspaceRootOutcome::default());
    };

    let cwd_values = thread_cwd_by_id
        .values()
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .collect::<Vec<_>>();
    let existing_saved_roots = to_path_array(state_object.get("electron-saved-workspace-roots"));
    let existing_project_order = to_path_array(state_object.get("project-order"));
    let existing_active_value = state_object.get("active-workspace-roots").cloned();
    let existing_active_roots = to_path_array(existing_active_value.as_ref());

    let root_sources = if existing_project_order.is_empty() {
        [&existing_saved_roots[..], &existing_active_roots[..]].concat()
    } else {
        [
            &existing_project_order[..],
            &existing_saved_roots[..],
            &existing_active_roots[..],
        ]
        .concat()
    };
    let next_saved_roots = dedupe_paths(
        root_sources
            .iter()
            .map(|value| resolve_stored_path(value, &cwd_values)),
    );
    let project_order_sources = if existing_project_order.is_empty() {
        next_saved_roots.clone()
    } else {
        [&existing_project_order[..], &existing_saved_roots[..]].concat()
    };
    let next_project_order = dedupe_paths(
        project_order_sources
            .iter()
            .map(|value| resolve_stored_path(value, &cwd_values)),
    );
    let next_active_roots = dedupe_paths(
        existing_active_roots
            .iter()
            .map(|value| resolve_stored_path(value, &cwd_values)),
    );
    let next_active_value = match existing_active_value.as_ref() {
        Some(Value::Array(_)) => Value::Array(
            next_active_roots
                .iter()
                .map(|value| Value::String(value.clone()))
                .collect(),
        ),
        Some(existing) => next_active_roots
            .first()
            .cloned()
            .map(Value::String)
            .unwrap_or_else(|| existing.clone()),
        None => Value::Array(Vec::new()),
    };

    let next_labels = copy_resolved_object_keys(
        state_object.get("electron-workspace-root-labels"),
        &cwd_values,
    );
    let next_open_targets =
        copy_open_target_preferences(state_object.get("open-in-target-preferences"), &cwd_values);
    let backup_path = source_home.join(GLOBAL_STATE_BACKUP_FILE_NAME);
    let backup_missing = !backup_path.exists();

    let saved_roots_changed = existing_saved_roots != next_saved_roots;
    let project_order_changed = existing_project_order != next_project_order;
    let active_roots_changed = existing_active_value.is_some()
        && existing_active_value.as_ref() != Some(&next_active_value);
    let labels_changed = value_option_changed(
        state_object.get("electron-workspace-root-labels"),
        next_labels.as_ref(),
    );
    let open_targets_changed = value_option_changed(
        state_object.get("open-in-target-preferences"),
        next_open_targets.as_ref(),
    );

    state_object.insert(
        "electron-saved-workspace-roots".to_string(),
        Value::Array(
            next_saved_roots
                .iter()
                .map(|value| Value::String(value.clone()))
                .collect(),
        ),
    );
    state_object.insert(
        "project-order".to_string(),
        Value::Array(
            next_project_order
                .iter()
                .map(|value| Value::String(value.clone()))
                .collect(),
        ),
    );
    if existing_active_value.is_some() {
        state_object.insert("active-workspace-roots".to_string(), next_active_value);
    }
    if let Some(labels) = next_labels {
        state_object.insert("electron-workspace-root-labels".to_string(), labels);
    }
    if let Some(open_targets) = next_open_targets {
        state_object.insert("open-in-target-preferences".to_string(), open_targets);
    }

    if saved_roots_changed
        || project_order_changed
        || active_roots_changed
        || labels_changed
        || open_targets_changed
        || backup_missing
    {
        let next_text = format!("{}\n", serde_json::to_string_pretty(&state)?);
        write_text_atomically(&state_path, &next_text)?;
        write_text_atomically(&backup_path, &next_text)?;
    }

    Ok(WorkspaceRootOutcome {
        updated_workspace_roots: count_array_changes(&existing_saved_roots, &next_saved_roots),
        saved_workspace_root_count: next_saved_roots.len(),
    })
}

fn to_path_array(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .collect(),
        Some(Value::String(value)) if !value.trim().is_empty() => {
            vec![value.trim().to_string()]
        }
        _ => Vec::new(),
    }
}

fn dedupe_paths(paths: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    for path in paths {
        let Some(comparable) = normalize_comparable_path(&path) else {
            continue;
        };
        if seen.insert(comparable) {
            result.push(path);
        }
    }
    result
}

fn resolve_stored_path(value: &str, cwd_values: &[String]) -> String {
    let Some(comparable) = normalize_comparable_path(value) else {
        return value.to_string();
    };
    cwd_values
        .iter()
        .find(|cwd| normalize_comparable_path(cwd).as_deref() == Some(comparable.as_str()))
        .map(|cwd| to_desktop_workspace_path(cwd))
        .unwrap_or_else(|| to_desktop_workspace_path(value))
}

fn normalize_comparable_path(value: &str) -> Option<String> {
    let mut normalized = value.trim().to_string();
    if normalized.is_empty() {
        return None;
    }
    if let Some(rest) = normalized.strip_prefix("\\\\?\\UNC\\") {
        normalized = format!("\\\\{rest}");
    } else if let Some(rest) = normalized.strip_prefix("\\\\?\\") {
        normalized = rest.to_string();
    }
    normalized = normalized.replace('/', "\\");
    while normalized.len() > 1 && normalized.ends_with('\\') {
        normalized.pop();
    }
    if normalized.len() == 2 && normalized.as_bytes()[1] == b':' {
        normalized.push('\\');
    }
    Some(normalized.to_ascii_lowercase())
}

fn to_desktop_workspace_path(value: &str) -> String {
    let trimmed = value.trim();
    if let Some(rest) = trimmed.strip_prefix("\\\\?\\UNC\\") {
        return format!("\\\\{}", rest.replace('/', "\\"));
    }
    if let Some(rest) = trimmed.strip_prefix("\\\\?\\") {
        return rest.replace('/', "\\");
    }
    value.to_string()
}

fn copy_resolved_object_keys(value: Option<&Value>, cwd_values: &[String]) -> Option<Value> {
    match value {
        Some(Value::Object(map)) => {
            let mut next = Map::new();
            for (key, value) in map {
                let resolved = resolve_stored_path(key, cwd_values);
                if !next.contains_key(&resolved) || resolved == *key {
                    next.insert(resolved, value.clone());
                }
            }
            Some(Value::Object(next))
        }
        Some(value) => Some(value.clone()),
        None => None,
    }
}

fn copy_open_target_preferences(value: Option<&Value>, cwd_values: &[String]) -> Option<Value> {
    match value {
        Some(Value::Object(map)) => {
            let mut next = map.clone();
            if let Some(per_path) = copy_resolved_object_keys(map.get("perPath"), cwd_values) {
                next.insert("perPath".to_string(), per_path);
            }
            Some(Value::Object(next))
        }
        Some(value) => Some(value.clone()),
        None => None,
    }
}

fn value_option_changed(left: Option<&Value>, right: Option<&Value>) -> bool {
    match (left, right) {
        (None, None) => false,
        (Some(left), Some(right)) => left != right,
        _ => true,
    }
}

fn count_array_changes(previous: &[String], next: &[String]) -> usize {
    let compared = previous.len().max(next.len());
    (0..compared)
        .filter(|index| previous.get(*index) != next.get(*index))
        .count()
}

fn write_text_atomically(path: &Path, contents: &str) -> ModexResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_file_name(format!(
        "{}.modex-provider-sync",
        path.file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("state")
    ));
    fs::write(&temporary, contents)?;
    if let Ok(metadata) = fs::metadata(path) {
        let _ = fs::set_permissions(&temporary, metadata.permissions());
    }
    fs::rename(&temporary, path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        ModexError::from(error)
    })?;
    Ok(())
}

fn is_rollout_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| name.starts_with("rollout-") && name.ends_with(".jsonl"))
}

fn extract_session_meta_payload_mut(value: &mut Value) -> Option<&mut Value> {
    if value
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind == "session_meta")
    {
        return value.get_mut("payload");
    }
    value
        .get_mut("session_meta")
        .and_then(|inner| inner.get_mut("payload"))
}

fn rollout_has_user_event(contents: &str) -> bool {
    contents
        .lines()
        .filter(|line| !line.trim().is_empty())
        .any(|line| {
            serde_json::from_str::<Value>(line)
                .ok()
                .is_some_and(|value| record_has_user_event(&value))
        })
}

fn record_has_user_event(record: &Value) -> bool {
    if record
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind == "event_msg")
        && record
            .get("payload")
            .and_then(|payload| payload.get("type"))
            .and_then(Value::as_str)
            .is_some_and(|kind| kind == "user_message")
    {
        return true;
    }

    ["payload", "item", "msg"].iter().any(|key| {
        record.get(*key).is_some_and(|value| {
            value.get("type").and_then(Value::as_str) == Some("message")
                && value.get("role").and_then(Value::as_str) == Some("user")
        })
    })
}

fn build_encrypted_content_warning(
    counts_by_provider: &BTreeMap<String, usize>,
    target_provider: &str,
    sanitized_encrypted_content_fields: usize,
) -> Option<String> {
    let risky_providers = counts_by_provider
        .iter()
        .filter_map(|(provider, count)| {
            (*count > 0 && provider != target_provider).then_some(provider.as_str())
        })
        .collect::<Vec<_>>();
    if risky_providers.is_empty() {
        return None;
    }
    let total = counts_by_provider.values().sum::<usize>();
    if sanitized_encrypted_content_fields > 0 {
        return Some(format!(
            "Modex removed {sanitized_encrypted_content_fields} incompatible encrypted_content field(s) from {total} rollout file(s) generated by provider(s) {} while switching history metadata to {target_provider}. The visible transcript was preserved so the thread can continue with the new account.",
            risky_providers.join(", ")
        ));
    }
    Some(format!(
        "Encrypted content warning: {total} rollout file(s) contain encrypted_content from provider(s) {}. Visibility metadata was synchronized to {target_provider}, but continuing or compacting those histories may fail with invalid_encrypted_content.",
        risky_providers.join(", ")
    ))
}

fn sqlite_error(error: rusqlite::Error) -> ModexError {
    ModexError::from(error.to_string())
}

#[derive(Clone, Debug)]
struct RolloutChange {
    path: PathBuf,
    original_first_line: String,
    original_separator: String,
    snapshot: FileSnapshot,
    updated_first_line: String,
    remove_encrypted_content: bool,
    original_contents: Option<Arc<str>>,
}

#[derive(Debug, Default)]
struct RolloutRewriteOutcome {
    applied: Vec<RolloutChange>,
    skipped: usize,
    sanitized_encrypted_content_fields: usize,
}

#[derive(Debug, Default)]
struct RolloutWriteOutcome {
    sanitized_encrypted_content_fields: usize,
}

#[derive(Debug, Default)]
struct SessionScan {
    changes: Vec<RolloutChange>,
    encrypted_content_counts: BTreeMap<String, usize>,
    user_event_thread_ids: BTreeSet<String>,
    thread_cwd_by_id: BTreeMap<String, String>,
}

#[derive(Debug, Default)]
struct SqliteUpdateOutcome {
    provider_rows_updated: usize,
    user_event_rows_updated: usize,
    cwd_rows_updated: usize,
}

#[derive(Debug, Default)]
struct WorkspaceRootOutcome {
    updated_workspace_roots: usize,
    saved_workspace_root_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileSnapshot {
    size: u64,
    modified: Option<SystemTime>,
}

impl FileSnapshot {
    fn capture(path: &Path) -> ModexResult<Self> {
        let metadata = fs::metadata(path)?;
        Ok(Self {
            size: metadata.len(),
            modified: metadata.modified().ok(),
        })
    }

    fn matches_path(self, path: &Path) -> ModexResult<bool> {
        Ok(self == Self::capture(path)?)
    }
}

struct FirstLineRecord {
    first_line: String,
    separator: String,
    rest: String,
}

impl FirstLineRecord {
    fn from_contents(contents: &str) -> Option<Self> {
        if contents.is_empty() {
            return None;
        }
        if let Some(newline_index) = contents.find('\n') {
            let mut first_line = contents[..newline_index].to_string();
            let separator = if first_line.ends_with('\r') {
                first_line.pop();
                "\r\n".to_string()
            } else {
                "\n".to_string()
            };
            return Some(Self {
                first_line,
                separator,
                rest: contents[newline_index + 1..].to_string(),
            });
        }
        Some(Self {
            first_line: contents.to_string(),
            separator: String::new(),
            rest: String::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn rollout_rewrite_skips_when_file_changes_after_scan() {
        let temp = tempfile::TempDir::new().unwrap();
        let rollout_path = temp
            .path()
            .join("rollout-2026-05-28T11-30-00-thread-race.jsonl");
        fs::write(
            &rollout_path,
            current_rollout_jsonl("openai", "thread-race", "/tmp/project-race"),
        )
        .unwrap();

        let mut scan = SessionScan::default();
        collect_rollout_change(&rollout_path, "modex-api-key", &mut scan).unwrap();
        fs::OpenOptions::new()
            .append(true)
            .open(&rollout_path)
            .unwrap()
            .write_all(br#"{"type":"response_item","payload":{"content":"new tail"}}"#)
            .unwrap();

        let outcome = apply_rollout_rewrites(&scan.changes).unwrap();

        assert_eq!(outcome.applied.len(), 0);
        assert_eq!(outcome.skipped, 1);
        let contents = fs::read_to_string(&rollout_path).unwrap();
        assert!(contents.contains("new tail"));
        let first_line = contents.lines().next().unwrap();
        let parsed = serde_json::from_str::<Value>(first_line).unwrap();
        assert_eq!(parsed["payload"]["model_provider"], "openai");
    }

    fn current_rollout_jsonl(provider: &str, thread_id: &str, cwd: &str) -> String {
        format!(
            "{}\n{}\n",
            serde_json::json!({
                "timestamp": "2026-05-28T03:30:00.000Z",
                "type": "session_meta",
                "payload": {
                    "id": thread_id,
                    "cwd": cwd,
                    "model_provider": provider
                }
            }),
            serde_json::json!({
                "timestamp": "2026-05-28T03:31:00.000Z",
                "type": "event_msg",
                "payload": {
                    "type": "user_message"
                }
            })
        )
    }
}

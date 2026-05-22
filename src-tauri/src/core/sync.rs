use std::collections::BTreeSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::Value;

use super::app_config::IdentityAuthType;
use super::{ModexError, ModexResult};

const STATE_DB_NAME: &str = "state_5.sqlite";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistorySyncProvider {
    OpenAi,
    ModexApiKey,
}

impl HistorySyncProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::ModexApiKey => "modex-api-key",
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

    let rollout_paths = source_history_rollout_paths(source_home)?;
    let mut connection = Connection::open(&state_path)?;
    let mut originals = Vec::new();
    let result = (|| {
        let transaction = connection.transaction()?;
        transaction.execute(
            "UPDATE threads SET model_provider = ?1",
            [provider.as_str()],
        )?;
        for rollout_path in rollout_paths {
            let original = rewrite_rollout_provider(&rollout_path, provider)?;
            originals.push((rollout_path, original));
        }
        transaction.commit()?;
        Ok(())
    })();
    if let Err(error) = result {
        if let Err(restore_error) = restore_rollout_contents(&originals) {
            return Err(ModexError::from(format!(
                "{error}; 且会话 rollout 回滚失败：{restore_error}"
            )));
        }
        return Err(error);
    }
    Ok(())
}

pub(crate) fn source_history_rollout_paths(source_home: &Path) -> ModexResult<Vec<PathBuf>> {
    let state_path = source_home.join(STATE_DB_NAME);
    if !state_path.exists() {
        return Ok(Vec::new());
    }

    let connection = Connection::open(&state_path)?;
    let mut statement = connection.prepare(
        "SELECT rollout_path
         FROM threads
         WHERE rollout_path IS NOT NULL
           AND TRIM(rollout_path) != ''
         ORDER BY rollout_path, id",
    )?;
    let mut rows = statement.query([])?;
    let mut rollout_paths = BTreeSet::new();
    while let Some(row) = rows.next()? {
        let rollout_path: String = row.get(0)?;
        rollout_paths.insert(resolve_rollout_path(source_home, &rollout_path));
    }
    Ok(rollout_paths.into_iter().collect())
}

fn resolve_rollout_path(source_home: &Path, rollout_path: &str) -> PathBuf {
    let path = PathBuf::from(rollout_path.trim());
    if path.is_absolute() {
        path
    } else {
        source_home.join(path)
    }
}

fn rewrite_rollout_provider(path: &Path, provider: HistorySyncProvider) -> ModexResult<String> {
    let content = fs::read_to_string(path).map_err(|error| {
        if error.kind() == ErrorKind::NotFound {
            ModexError::from(format!("会话 rollout 文件不存在：{}", path.display()))
        } else {
            error.into()
        }
    })?;
    if content.is_empty() {
        return Err(ModexError::from(format!(
            "会话 rollout 文件为空：{}",
            path.display()
        )));
    }

    let split_index = content
        .find('\n')
        .map(|index| index + 1)
        .unwrap_or(content.len());
    let first_segment = &content[..split_index];
    let rest = &content[split_index..];
    let line_ending = if first_segment.ends_with("\r\n") {
        "\r\n"
    } else if first_segment.ends_with('\n') {
        "\n"
    } else {
        ""
    };
    let first_line = first_segment.trim_end_matches(&['\r', '\n'][..]);
    let mut payload: Value = serde_json::from_str(first_line).map_err(|error| {
        ModexError::from(format!(
            "会话 rollout 首行不是合法 JSON：{} ({error})",
            path.display()
        ))
    })?;
    let Some(session_meta) = payload
        .get_mut("session_meta")
        .and_then(Value::as_object_mut)
    else {
        return Err(ModexError::from(format!(
            "会话 rollout 缺少 session_meta：{}",
            path.display()
        )));
    };
    let Some(meta_payload) = session_meta
        .get_mut("payload")
        .and_then(Value::as_object_mut)
    else {
        return Err(ModexError::from(format!(
            "会话 rollout 缺少 session_meta.payload：{}",
            path.display()
        )));
    };
    meta_payload.insert(
        "model_provider".to_string(),
        Value::String(provider.as_str().to_string()),
    );
    let first_line = serde_json::to_string(&payload)?;
    write_text_atomically(path, &format!("{first_line}{line_ending}{rest}"))?;
    Ok(content)
}

fn restore_rollout_contents(originals: &[(PathBuf, String)]) -> ModexResult<()> {
    for (path, content) in originals {
        write_text_atomically(path, content)?;
    }
    Ok(())
}

fn write_text_atomically(path: &Path, content: &str) -> ModexResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| ModexError::from(format!("无效文件路径：{}", path.display())))?;
    fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| ModexError::from(format!("无效文件路径：{}", path.display())))?;
    let temporary = parent.join(format!("{file_name}.modex-tmp"));
    fs::write(&temporary, content)?;
    if let Ok(metadata) = fs::metadata(path) {
        let _ = fs::set_permissions(&temporary, metadata.permissions());
    }
    fs::rename(temporary, path)?;
    Ok(())
}

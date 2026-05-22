use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

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

    let mut connection = Connection::open(&state_path)?;
    let transaction = connection.transaction()?;
    transaction.execute(
        "UPDATE threads SET model_provider = ?1",
        [provider.as_str()],
    )?;
    transaction.commit()?;
    Ok(())
}

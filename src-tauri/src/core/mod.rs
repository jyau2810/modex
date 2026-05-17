pub mod app_config;
pub mod auth;
pub mod codex;
pub mod engine;
pub mod identity_home;
pub mod quota;
pub mod sync;

use thiserror::Error;

pub type ModexResult<T> = Result<T, ModexError>;

#[derive(Debug, Error)]
pub enum ModexError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl From<&str> for ModexError {
    fn from(value: &str) -> Self {
        Self::Message(value.to_string())
    }
}

impl From<String> for ModexError {
    fn from(value: String) -> Self {
        Self::Message(value)
    }
}

use std::path::{Path, PathBuf};

use super::app_config::AppIdentity;
use super::{ModexError, ModexResult};

pub const MANAGED_HOME_DIR: &str = ".modex";
pub const RANDOM_HOME_DIGITS: usize = 12;

pub fn default_new_identity(
    home: &Path,
    mut random_digits: impl FnMut() -> String,
) -> ModexResult<AppIdentity> {
    for _ in 0..100 {
        let digits = random_digits();
        if digits.len() != RANDOM_HOME_DIGITS
            || !digits.chars().all(|character| character.is_ascii_digit())
        {
            continue;
        }
        let codex_home = home.join(MANAGED_HOME_DIR).join(digits);
        if !codex_home.exists() {
            return Ok(AppIdentity {
                name: "登录中".to_string(),
                codex_home,
                monitor: false,
                workspace_id: None,
                auth_type: Default::default(),
                api_base_url: None,
            });
        }
    }
    Err(ModexError::from("无法生成唯一账号配置目录"))
}

pub fn random_digits() -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    (0..RANDOM_HOME_DIGITS)
        .map(|_| char::from(b'0' + rng.random_range(0..10)))
        .collect()
}

pub fn is_managed_identity_home(path: &Path, home: &Path) -> bool {
    let expanded = normalize(path);
    let root = normalize(home);
    match expanded.parent() {
        Some(parent) if parent == root.join(MANAGED_HOME_DIR) => expanded
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                !name.is_empty() && name.chars().all(|character| character.is_ascii_digit())
            }),
        _ => false,
    }
}

fn normalize(path: &Path) -> PathBuf {
    path.components().collect()
}

use std::fs;
use std::path::{Path, PathBuf};

use super::{ModexError, ModexResult};

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

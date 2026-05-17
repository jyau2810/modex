use std::path::PathBuf;

use assert_fs::prelude::*;
use modex_lib::core::app_config::{load_app_settings_from_path, AppIdentity, AppSettings};
use modex_lib::core::engine::{AppEngine, SettingsPatch};

fn jwt_with_claims(claims: serde_json::Value) -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"none"}"#);
    let payload = URL_SAFE_NO_PAD.encode(claims.to_string());
    format!("{header}.{payload}.signature")
}

#[test]
fn add_identity_persists_new_managed_identity() {
    let temp = assert_fs::TempDir::new().unwrap();
    let config = temp.child("config.json");
    let mut engine = AppEngine::new(
        AppSettings::default_for_home(temp.path().to_path_buf()),
        config.path().to_path_buf(),
    );

    let identity = engine
        .add_identity_with_digits(|| "123456789012".to_string())
        .unwrap();

    assert_eq!(identity.name, "登录中");
    assert_eq!(identity.codex_home, temp.path().join(".modex/123456789012"));
    let saved = load_app_settings_from_path(config.path()).unwrap();
    assert_eq!(saved.identities.len(), 1);
    assert!(saved.has_completed_setup);
}

#[test]
fn update_settings_persists_patch_without_dropping_identities() {
    let temp = assert_fs::TempDir::new().unwrap();
    let config = temp.child("config.json");
    let mut settings = AppSettings::default_for_home(temp.path().to_path_buf());
    settings.identities.push(AppIdentity {
        name: "Team".to_string(),
        codex_home: PathBuf::from("/tmp/team"),
        monitor: true,
        workspace_id: None,
    });
    let mut engine = AppEngine::new(settings, config.path().to_path_buf());

    let updated = engine
        .update_settings(SettingsPatch {
            codex_binary: Some("/opt/codex".to_string()),
            app_name: None,
            poll_seconds: Some(90),
            source_home: None,
        })
        .unwrap();

    assert_eq!(updated.codex_binary, "/opt/codex");
    assert_eq!(updated.poll_seconds, 90);
    assert_eq!(updated.identities.len(), 1);
    let saved = load_app_settings_from_path(config.path()).unwrap();
    assert_eq!(saved.codex_binary, "/opt/codex");
}

#[test]
fn delete_identity_clears_current_identity_when_deleted() {
    let temp = assert_fs::TempDir::new().unwrap();
    let config = temp.child("config.json");
    let mut settings = AppSettings::default_for_home(temp.path().to_path_buf());
    settings.current_identity_name = Some("Team".to_string());
    settings.identities.push(AppIdentity {
        name: "Team".to_string(),
        codex_home: PathBuf::from("/tmp/team"),
        monitor: false,
        workspace_id: None,
    });
    let mut engine = AppEngine::new(settings, config.path().to_path_buf());

    engine.delete_identity("Team").unwrap();

    assert!(engine.settings().identities.is_empty());
    assert!(engine.settings().current_identity_name.is_none());
}

#[test]
fn sync_identity_names_from_auth_persists_browser_login_result() {
    let temp = assert_fs::TempDir::new().unwrap();
    let config = temp.child("config.json");
    let codex_home = temp.path().join(".modex/333333333333");
    let mut settings = AppSettings::default_for_home(temp.path().to_path_buf());
    settings.identities.push(AppIdentity {
        name: "登录中".to_string(),
        codex_home: codex_home.clone(),
        monitor: false,
        workspace_id: None,
    });
    let mut engine = AppEngine::new(settings, config.path().to_path_buf());
    std::fs::create_dir_all(&codex_home).unwrap();
    let token = jwt_with_claims(serde_json::json!({
        "email": "team@example.com",
        "https://api.openai.com/auth": {
            "chatgpt_plan_type": "team"
        }
    }));
    std::fs::write(
        codex_home.join("auth.json"),
        serde_json::json!({
            "tokens": {
                "id_token": token
            }
        })
        .to_string(),
    )
    .unwrap();

    let changed = engine.sync_identity_names_from_auth().unwrap();

    assert!(changed);
    assert_eq!(
        engine.settings().identities[0].name,
        "team@example.com · 团队版"
    );
    let saved = load_app_settings_from_path(config.path()).unwrap();
    assert_eq!(saved.identities[0].name, "team@example.com · 团队版");
}

#[test]
fn refresh_all_updates_identities_even_when_monitor_is_disabled() {
    let temp = assert_fs::TempDir::new().unwrap();
    let config = temp.child("config.json");
    let mut settings = AppSettings::default_for_home(temp.path().to_path_buf());
    settings.identities.push(AppIdentity {
        name: "Disabled monitor".to_string(),
        codex_home: temp.path().join("disabled-monitor"),
        monitor: false,
        workspace_id: None,
    });
    settings.identities.push(AppIdentity {
        name: "Enabled monitor".to_string(),
        codex_home: temp.path().join("enabled-monitor"),
        monitor: true,
        workspace_id: None,
    });
    let mut engine = AppEngine::new(settings, config.path().to_path_buf());

    let identities = engine.refresh_all();

    assert_eq!(identities.len(), 2);
    for identity in identities {
        assert_eq!(identity.quota.status, "error");
        assert!(identity
            .quota
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("账号缺少登录凭据"));
    }
}

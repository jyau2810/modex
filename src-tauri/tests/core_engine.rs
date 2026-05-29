use std::path::PathBuf;

use assert_fs::prelude::*;
use modex_lib::core::app_config::{
    load_app_settings_from_path, AppIdentity, AppSettings, IdentityAuthType,
};
use modex_lib::core::engine::{AppEngine, SettingsPatch};

fn jwt_with_claims(claims: serde_json::Value) -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"none"}"#);
    let payload = URL_SAFE_NO_PAD.encode(claims.to_string());
    format!("{header}.{payload}.signature")
}

fn auth_json(email: &str, sub: &str, account_id: &str, plan_type: &str) -> String {
    let token = jwt_with_claims(serde_json::json!({
        "email": email,
        "sub": sub,
        "https://api.openai.com/auth": {
            "account_id": account_id,
            "chatgpt_plan_type": plan_type
        }
    }));
    serde_json::json!({
        "tokens": {
            "account_id": account_id,
            "id_token": token
        }
    })
    .to_string()
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
    assert_eq!(
        identity.codex_home,
        temp.path()
            .join(".modex/123456789012")
            .display()
            .to_string()
    );
    let saved = load_app_settings_from_path(config.path()).unwrap();
    assert_eq!(saved.identities.len(), 1);
    assert!(saved.has_completed_setup);
}

#[test]
fn add_api_key_identity_creates_isolated_api_key_account() {
    let temp = assert_fs::TempDir::new().unwrap();
    let config = temp.child("config.json");
    let mut engine = AppEngine::new(
        AppSettings::default_for_home(temp.path().to_path_buf()),
        config.path().to_path_buf(),
    );
    let mut login_home = None;
    let mut login_key = None;

    let identity = engine
        .add_api_key_identity_with_operations(
            " Gateway ",
            " sk-test-key ",
            Some(" https://gateway.example/v1 ".to_string()),
            || "123456789012".to_string(),
            |_settings, identity, api_key| {
                login_home = Some(identity.codex_home.clone());
                login_key = Some(api_key.to_string());
                std::fs::create_dir_all(&identity.codex_home).unwrap();
                std::fs::write(
                    identity.codex_home.join("auth.json"),
                    r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-test-key"}"#,
                )
                .unwrap();
                Ok(())
            },
        )
        .unwrap();

    assert_eq!(identity.name, "Gateway");
    assert_eq!(identity.auth_type, IdentityAuthType::ApiKey);
    assert_eq!(
        identity.api_base_url.as_deref(),
        Some("https://gateway.example/v1")
    );
    assert!(identity.logged_in);
    assert_eq!(identity.quota.status, "unknown");
    assert_eq!(identity.quota.plan, "计划未知");
    assert_eq!(identity.quota.primary_percent, 0);
    assert_eq!(identity.quota.secondary_percent, 0);
    assert_eq!(login_home.unwrap(), temp.path().join(".modex/123456789012"));
    assert_eq!(login_key.as_deref(), Some("sk-test-key"));

    let saved = load_app_settings_from_path(config.path()).unwrap();
    assert_eq!(saved.identities[0].name, "Gateway");
    assert_eq!(saved.identities[0].auth_type, IdentityAuthType::ApiKey);
    assert_eq!(
        saved.identities[0].api_base_url.as_deref(),
        Some("https://gateway.example/v1")
    );
}

#[test]
fn add_api_key_identity_rejects_empty_name() {
    let temp = assert_fs::TempDir::new().unwrap();
    let config = temp.child("config.json");
    let mut engine = AppEngine::new(
        AppSettings::default_for_home(temp.path().to_path_buf()),
        config.path().to_path_buf(),
    );

    let empty_name = engine.add_api_key_identity_with_operations(
        " ",
        "sk-test-key",
        None,
        || "123456789012".to_string(),
        |_settings, _identity, _api_key| Ok(()),
    );

    assert!(empty_name
        .unwrap_err()
        .to_string()
        .contains("账号名不能为空"));
    assert!(engine.settings().identities.is_empty());
}

#[test]
fn add_api_key_identity_rejects_empty_key() {
    let temp = assert_fs::TempDir::new().unwrap();
    let config = temp.child("config.json");
    let mut engine = AppEngine::new(
        AppSettings::default_for_home(temp.path().to_path_buf()),
        config.path().to_path_buf(),
    );

    let empty_key = engine.add_api_key_identity_with_operations(
        "Gateway",
        " ",
        None,
        || "123456789012".to_string(),
        |_settings, _identity, _api_key| Ok(()),
    );

    assert!(empty_key
        .unwrap_err()
        .to_string()
        .contains("API Key 不能为空"));
    assert!(engine.settings().identities.is_empty());
}

#[test]
fn sync_current_identity_from_source_auth_detects_api_key_identity() {
    let temp = assert_fs::TempDir::new().unwrap();
    let config = temp.child("config.json");
    let source_home = temp.path().join("source");
    let api_home = temp.path().join(".modex/api");
    std::fs::create_dir_all(&source_home).unwrap();
    std::fs::create_dir_all(&api_home).unwrap();
    let auth = r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-test"}"#;
    std::fs::write(source_home.join("auth.json"), auth).unwrap();
    std::fs::write(api_home.join("auth.json"), auth).unwrap();
    let mut settings = AppSettings::default_for_home(temp.path().to_path_buf());
    settings.source_home = source_home;
    settings.identities.push(AppIdentity {
        name: "Gateway".to_string(),
        codex_home: api_home,
        monitor: false,
        workspace_id: None,
        auth_type: IdentityAuthType::ApiKey,
        api_base_url: Some("https://gateway.example/v1".to_string()),
    });
    let mut engine = AppEngine::new(settings, config.path().to_path_buf());

    assert!(engine.sync_current_identity_from_source_auth().unwrap());

    assert_eq!(
        engine.settings().current_identity_name.as_deref(),
        Some("Gateway")
    );
    assert!(engine.app_state().identities[0].is_current);
}

#[test]
fn refresh_api_key_identity_skips_quota_reader() {
    let temp = assert_fs::TempDir::new().unwrap();
    let config = temp.child("config.json");
    let api_home = temp.path().join(".modex/api");
    std::fs::create_dir_all(&api_home).unwrap();
    std::fs::write(
        api_home.join("auth.json"),
        r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-test"}"#,
    )
    .unwrap();
    let mut settings = AppSettings::default_for_home(temp.path().to_path_buf());
    settings.identities.push(AppIdentity {
        name: "Gateway".to_string(),
        codex_home: api_home,
        monitor: false,
        workspace_id: None,
        auth_type: IdentityAuthType::ApiKey,
        api_base_url: None,
    });
    let mut engine = AppEngine::new(settings, config.path().to_path_buf());

    let identity = engine
        .refresh_identity_with_reader("Gateway", |_settings, _identity| {
            panic!("API Key identities should not query quota during refresh")
        })
        .unwrap();

    assert!(identity.logged_in);
    assert!(!identity.login_expired);
    assert_eq!(identity.quota.status, "unknown");
    assert_eq!(identity.quota.plan, "计划未知");
}

#[test]
fn api_key_quota_errors_do_not_mark_login_expired() {
    let temp = assert_fs::TempDir::new().unwrap();
    let config = temp.child("config.json");
    let api_home = temp.path().join(".modex/api");
    std::fs::create_dir_all(&api_home).unwrap();
    std::fs::write(
        api_home.join("auth.json"),
        r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-test"}"#,
    )
    .unwrap();
    let mut settings = AppSettings::default_for_home(temp.path().to_path_buf());
    settings.identities.push(AppIdentity {
        name: "Gateway".to_string(),
        codex_home: api_home,
        monitor: false,
        workspace_id: None,
        auth_type: IdentityAuthType::ApiKey,
        api_base_url: None,
    });
    let mut engine = AppEngine::new(settings, config.path().to_path_buf());

    engine.set_error("Gateway", "unauthorized".to_string());

    let identity = engine.app_state().identities.into_iter().next().unwrap();
    assert!(identity.logged_in);
    assert!(!identity.login_expired);
    assert_eq!(identity.quota.status, "error");
}

#[test]
fn chatgpt_token_expired_error_marks_login_expired() {
    let temp = assert_fs::TempDir::new().unwrap();
    let config = temp.child("config.json");
    let chatgpt_home = temp.path().join(".modex/chatgpt");
    std::fs::create_dir_all(&chatgpt_home).unwrap();
    std::fs::write(
        chatgpt_home.join("auth.json"),
        auth_json("team@example.com", "user-team", "acct-team", "team"),
    )
    .unwrap();
    let mut settings = AppSettings::default_for_home(temp.path().to_path_buf());
    settings.identities.push(AppIdentity {
        name: "Team".to_string(),
        codex_home: chatgpt_home,
        monitor: false,
        workspace_id: None,
        auth_type: Default::default(),
        api_base_url: None,
    });
    let mut engine = AppEngine::new(settings, config.path().to_path_buf());

    engine.set_error(
        "team@example.com · 团队版",
        r#"{"code":"token_expired"}"#.to_string(),
    );

    let identity = engine.app_state().identities.into_iter().next().unwrap();
    assert!(identity.login_expired);
    assert!(!identity.logged_in);
    assert_eq!(identity.quota.status, "error");
}

#[test]
fn import_current_identity_copies_only_source_auth_to_managed_home() {
    let temp = assert_fs::TempDir::new().unwrap();
    let config = temp.child("config.json");
    let source_home = temp.path().join("source");
    std::fs::create_dir_all(&source_home).unwrap();
    let auth = auth_json("yaoji@example.com", "user-yaoji", "acct-team", "team");
    std::fs::write(source_home.join("auth.json"), &auth).unwrap();
    std::fs::write(source_home.join("config.toml"), "model = 'keep-out'\n").unwrap();
    std::fs::write(source_home.join("state_5.sqlite"), "runtime").unwrap();
    std::fs::create_dir(source_home.join("logs")).unwrap();
    std::fs::write(source_home.join("logs/codex.log"), "runtime log").unwrap();
    let mut settings = AppSettings::default_for_home(temp.path().to_path_buf());
    settings.source_home = source_home;
    let mut engine = AppEngine::new(settings, config.path().to_path_buf());

    let result = engine
        .import_current_identity_with_digits(|| "123456789012".to_string())
        .unwrap();

    assert!(result.ok);
    assert!(result.imported);
    let identity = result.identity.unwrap();
    assert_eq!(identity.name, "yaoji@example.com · 团队版");
    assert!(identity.logged_in);
    let imported_home = temp.path().join(".modex/123456789012");
    assert_eq!(identity.codex_home, imported_home.display().to_string());
    assert_eq!(
        std::fs::read_to_string(imported_home.join("auth.json")).unwrap(),
        auth
    );
    let imported_files = std::fs::read_dir(&imported_home)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    assert_eq!(imported_files, vec!["auth.json"]);
    assert_eq!(
        engine.settings().current_identity_name.as_deref(),
        Some("yaoji@example.com · 团队版")
    );
    let saved = load_app_settings_from_path(config.path()).unwrap();
    assert_eq!(saved.identities.len(), 1);
    assert!(saved.has_completed_setup);
}

#[test]
fn import_current_identity_reuses_existing_matching_account() {
    let temp = assert_fs::TempDir::new().unwrap();
    let config = temp.child("config.json");
    let source_home = temp.path().join("source");
    let existing_home = temp.path().join(".modex/111111111111");
    std::fs::create_dir_all(&source_home).unwrap();
    std::fs::create_dir_all(&existing_home).unwrap();
    let auth = auth_json("same@example.com", "user-same", "acct-same", "team");
    std::fs::write(source_home.join("auth.json"), &auth).unwrap();
    std::fs::write(existing_home.join("auth.json"), &auth).unwrap();
    let mut settings = AppSettings::default_for_home(temp.path().to_path_buf());
    settings.source_home = source_home;
    settings.current_identity_name = Some("other@example.com".to_string());
    settings.has_completed_setup = true;
    settings.identities.push(AppIdentity {
        name: "same@example.com · 团队版".to_string(),
        codex_home: existing_home,
        monitor: false,
        workspace_id: None,
        auth_type: Default::default(),
        api_base_url: None,
    });
    let mut engine = AppEngine::new(settings, config.path().to_path_buf());

    let result = engine
        .import_current_identity_with_digits(|| "222222222222".to_string())
        .unwrap();

    assert!(result.ok);
    assert!(!result.imported);
    assert_eq!(result.identity.unwrap().name, "same@example.com · 团队版");
    assert_eq!(engine.settings().identities.len(), 1);
    assert!(!temp.path().join(".modex/222222222222").exists());
    assert_eq!(
        engine.settings().current_identity_name.as_deref(),
        Some("same@example.com · 团队版")
    );
}

#[test]
fn import_current_identity_marks_new_import_as_current_even_when_accounts_exist() {
    let temp = assert_fs::TempDir::new().unwrap();
    let config = temp.child("config.json");
    let source_home = temp.path().join("source");
    let existing_home = temp.path().join(".modex/111111111111");
    std::fs::create_dir_all(&source_home).unwrap();
    std::fs::create_dir_all(&existing_home).unwrap();
    std::fs::write(
        source_home.join("auth.json"),
        auth_json("new@example.com", "user-new", "acct-new", "team"),
    )
    .unwrap();
    std::fs::write(
        existing_home.join("auth.json"),
        auth_json("old@example.com", "user-old", "acct-old", "team"),
    )
    .unwrap();
    let mut settings = AppSettings::default_for_home(temp.path().to_path_buf());
    settings.source_home = source_home;
    settings.current_identity_name = Some("old@example.com · 团队版".to_string());
    settings.has_completed_setup = true;
    settings.identities.push(AppIdentity {
        name: "old@example.com · 团队版".to_string(),
        codex_home: existing_home,
        monitor: false,
        workspace_id: None,
        auth_type: Default::default(),
        api_base_url: None,
    });
    let mut engine = AppEngine::new(settings, config.path().to_path_buf());

    let result = engine
        .import_current_identity_with_digits(|| "222222222222".to_string())
        .unwrap();

    assert!(result.imported);
    assert_eq!(
        engine.settings().current_identity_name.as_deref(),
        Some("new@example.com · 团队版")
    );
    assert!(result.identity.unwrap().is_current);
}

#[test]
fn import_current_identity_without_source_auth_leaves_settings_unchanged() {
    let temp = assert_fs::TempDir::new().unwrap();
    let config = temp.child("config.json");
    let source_home = temp.path().join("source");
    std::fs::create_dir_all(&source_home).unwrap();
    let mut settings = AppSettings::default_for_home(temp.path().to_path_buf());
    settings.source_home = source_home;
    settings.has_completed_setup = true;
    let mut engine = AppEngine::new(settings, config.path().to_path_buf());

    let result = engine
        .import_current_identity_with_digits(|| "123456789012".to_string())
        .unwrap();

    assert!(!result.ok);
    assert!(!result.imported);
    assert!(result.identity.is_none());
    assert!(result.message.contains("尚未登录"));
    assert!(engine.settings().identities.is_empty());
    assert!(engine.settings().has_completed_setup);
    assert!(!config.path().exists());
}

#[test]
fn import_current_identity_with_unparseable_auth_uses_fallback_account_name() {
    let temp = assert_fs::TempDir::new().unwrap();
    let config = temp.child("config.json");
    let source_home = temp.path().join("source");
    std::fs::create_dir_all(&source_home).unwrap();
    std::fs::write(
        source_home.join("auth.json"),
        serde_json::json!({"tokens": {"id_token": "not-a-jwt"}}).to_string(),
    )
    .unwrap();
    let mut settings = AppSettings::default_for_home(temp.path().to_path_buf());
    settings.source_home = source_home;
    let mut engine = AppEngine::new(settings, config.path().to_path_buf());

    let result = engine
        .import_current_identity_with_digits(|| "123456789012".to_string())
        .unwrap();

    assert!(result.ok);
    assert!(result.imported);
    assert_eq!(result.identity.unwrap().name, "账号");
    assert_eq!(engine.settings().identities[0].name, "账号");
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
        auth_type: Default::default(),
        api_base_url: None,
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
        auth_type: Default::default(),
        api_base_url: None,
    });
    let mut engine = AppEngine::new(settings, config.path().to_path_buf());

    engine.delete_identity("Team").unwrap();

    assert!(engine.settings().identities.is_empty());
    assert!(engine.settings().current_identity_name.is_none());
}

#[test]
fn delete_identity_removes_managed_identity_home() {
    let temp = assert_fs::TempDir::new().unwrap();
    let config = temp.child("config.json");
    let identity_home = temp.path().join(".modex/123456789012");
    std::fs::create_dir_all(identity_home.join("sessions")).unwrap();
    std::fs::write(identity_home.join("auth.json"), "{}").unwrap();
    std::fs::write(identity_home.join("sessions/rollout.jsonl"), "{}").unwrap();
    let mut settings = AppSettings::default_for_home(temp.path().to_path_buf());
    settings.identities.push(AppIdentity {
        name: "Team".to_string(),
        codex_home: identity_home.clone(),
        monitor: false,
        workspace_id: None,
        auth_type: Default::default(),
        api_base_url: None,
    });
    let mut engine = AppEngine::new(settings, config.path().to_path_buf());

    engine.delete_identity("Team").unwrap();

    assert!(!identity_home.exists());
}

#[test]
fn cleanup_unreferenced_managed_homes_removes_only_orphans() {
    let temp = assert_fs::TempDir::new().unwrap();
    let config = temp.child("config.json");
    let referenced_home = temp.path().join(".modex/111111111111");
    let orphan_home = temp.path().join(".modex/222222222222");
    let source_home = temp.path().join(".codex");
    let non_managed_home = temp.path().join(".modex/not-managed");
    std::fs::create_dir_all(&source_home).unwrap();
    for home in [&referenced_home, &orphan_home, &non_managed_home] {
        std::fs::create_dir_all(home).unwrap();
        std::fs::write(home.join("auth.json"), "{}").unwrap();
    }
    std::fs::create_dir_all(orphan_home.join("sessions")).unwrap();
    std::fs::write(orphan_home.join("sessions/rollout.jsonl"), "{}").unwrap();
    let mut settings = AppSettings::default_for_home(temp.path().to_path_buf());
    settings.source_home = source_home.clone();
    settings.identities.push(AppIdentity {
        name: "Team".to_string(),
        codex_home: referenced_home.clone(),
        monitor: false,
        workspace_id: None,
        auth_type: Default::default(),
        api_base_url: None,
    });
    let engine = AppEngine::new(settings, config.path().to_path_buf());

    let removed = engine.cleanup_unreferenced_managed_homes().unwrap();

    assert_eq!(removed, vec![orphan_home.clone()]);
    assert!(referenced_home.exists());
    assert!(!orphan_home.exists());
    assert!(source_home.exists());
    assert!(non_managed_home.exists());
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
        auth_type: Default::default(),
        api_base_url: None,
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
        auth_type: Default::default(),
        api_base_url: None,
    });
    settings.identities.push(AppIdentity {
        name: "Enabled monitor".to_string(),
        codex_home: temp.path().join("enabled-monitor"),
        monitor: true,
        workspace_id: None,
        auth_type: Default::default(),
        api_base_url: None,
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

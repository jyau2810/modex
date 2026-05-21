use std::path::PathBuf;

use assert_fs::prelude::*;
use modex_lib::core::app_config::AppIdentity;
use modex_lib::core::auth::{auth_identity_display_name, unique_identity_name};
use modex_lib::core::identity_home::{default_new_identity, is_managed_identity_home};

fn jwt_with_claims(claims: serde_json::Value) -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"none"}"#);
    let payload = URL_SAFE_NO_PAD.encode(claims.to_string());
    format!("{header}.{payload}.signature")
}

#[test]
fn default_new_identity_uses_modex_managed_home_with_12_digits() {
    let temp = assert_fs::TempDir::new().unwrap();

    let identity = default_new_identity(temp.path(), || "123456789012".to_string()).unwrap();

    assert_eq!(identity.name, "登录中");
    assert_eq!(identity.codex_home, temp.path().join(".modex/123456789012"));
    assert!(is_managed_identity_home(&identity.codex_home, temp.path()));
}

#[test]
fn auth_display_name_prefers_email_from_id_token() {
    let temp = assert_fs::TempDir::new().unwrap();
    let auth = temp.child("auth.json");
    let token = jwt_with_claims(serde_json::json!({
        "email": "team@example.com",
        "sub": "user-1",
        "https://api.openai.com/auth": {
            "chatgpt_plan_type": "team"
        }
    }));
    auth.write_str(
        &serde_json::json!({
            "tokens": {
                "account_id": "acct-team",
                "id_token": token
            }
        })
        .to_string(),
    )
    .unwrap();

    let name = auth_identity_display_name(temp.path()).unwrap();

    assert_eq!(name, "team@example.com · 团队版");
}

#[test]
fn unique_identity_name_adds_suffix_for_collisions() {
    let existing = vec![
        AppIdentity {
            name: "team@example.com".to_string(),
            codex_home: PathBuf::from("/tmp/a"),
            monitor: false,
            workspace_id: None,
            auth_type: Default::default(),
            api_base_url: None,
        },
        AppIdentity {
            name: "team@example.com 2".to_string(),
            codex_home: PathBuf::from("/tmp/b"),
            monitor: false,
            workspace_id: None,
            auth_type: Default::default(),
            api_base_url: None,
        },
    ];

    let name = unique_identity_name(
        "team@example.com",
        existing.iter().map(|identity| identity.name.as_str()),
    );

    assert_eq!(name, "team@example.com 3");
}

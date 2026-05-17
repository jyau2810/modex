use std::path::PathBuf;

use assert_fs::prelude::*;
use modex_lib::core::app_config::{
    load_app_settings_from_path, save_app_settings_to_path, AppIdentity, AppSettings,
};

#[test]
fn config_roundtrip_preserves_existing_fields() {
    let temp = assert_fs::TempDir::new().unwrap();
    let config = temp.child("config.json");
    config
        .write_str(
            r#"{
  "version": 1,
  "codexBinary": "/usr/local/bin/codex",
  "appName": "Codex",
  "pollSeconds": 45,
  "sourceHome": "/Users/alex/.codex",
  "hasCompletedSetup": true,
  "currentIdentityName": "team@example.com",
  "identities": [
    {
      "name": "team@example.com",
      "codexHome": "/Users/alex/.modex/123456789012",
      "monitor": true,
      "workspaceId": "workspace-a"
    }
  ]
}"#,
        )
        .unwrap();

    let settings = load_app_settings_from_path(config.path()).unwrap();

    assert_eq!(settings.codex_binary, "/usr/local/bin/codex");
    assert_eq!(settings.poll_seconds, 45);
    assert_eq!(
        settings.current_identity_name.as_deref(),
        Some("team@example.com")
    );
    assert_eq!(
        settings.identities[0].workspace_id.as_deref(),
        Some("workspace-a")
    );

    save_app_settings_to_path(&settings, config.path()).unwrap();
    let saved = load_app_settings_from_path(config.path()).unwrap();

    assert_eq!(saved, settings);
}

#[test]
fn config_defaults_match_modex_conventions() {
    let settings = AppSettings::default_for_home(PathBuf::from("/Users/alex"));

    assert_eq!(settings.codex_binary, "codex");
    assert_eq!(settings.app_name, "Codex");
    assert_eq!(settings.poll_seconds, 60);
    assert_eq!(settings.source_home, PathBuf::from("/Users/alex/.codex"));
    assert!(settings.identities.is_empty());
}

#[test]
fn app_identity_serializes_camel_case_paths() {
    let identity = AppIdentity {
        name: "Account".to_string(),
        codex_home: PathBuf::from("/tmp/account"),
        monitor: true,
        workspace_id: None,
    };

    let value = serde_json::to_value(identity).unwrap();

    assert_eq!(value["codexHome"], "/tmp/account");
    assert_eq!(value["workspaceId"], serde_json::Value::Null);
}

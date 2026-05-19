use std::path::PathBuf;

use assert_fs::prelude::*;
use modex_lib::core::app_config::{
    load_app_settings_from_path, save_app_settings_to_path, AppIdentity, AppSettings,
    DailyWakeSettings,
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
    assert_eq!(settings.daily_wake, DailyWakeSettings::default());
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
    assert_eq!(settings.daily_wake, DailyWakeSettings::default());
    assert!(settings.identities.is_empty());
}

#[test]
fn daily_wake_settings_roundtrip_custom_values() {
    let temp = assert_fs::TempDir::new().unwrap();
    let config = temp.child("config.json");
    config
        .write_str(
            r#"{
  "version": 1,
  "codexBinary": "codex",
  "appName": "Codex",
  "pollSeconds": 60,
  "sourceHome": "/Users/alex/.codex",
  "dailyWake": {
    "enabled": true,
    "time": "09:15",
    "times": ["09:15", "14:00"],
    "message": "Good morning",
    "skipIfPrimaryUsedAbovePercent": 3,
    "skipIfWeeklyRemainingBelowPercent": 20,
    "maxPrimaryDeltaPercent": 3,
    "lastRunDate": "2026-05-18",
    "lastRunSlots": ["2026-05-18#09:15"]
  },
  "identities": []
}"#,
        )
        .unwrap();

    let settings = load_app_settings_from_path(config.path()).unwrap();

    assert!(settings.daily_wake.enabled);
    assert_eq!(settings.daily_wake.time, "09:15");
    assert_eq!(settings.daily_wake.times, vec!["09:15", "14:00"]);
    assert_eq!(settings.daily_wake.message, "Good morning");
    assert_eq!(settings.daily_wake.skip_if_primary_used_above_percent, 3);
    assert_eq!(
        settings.daily_wake.skip_if_weekly_remaining_below_percent,
        20
    );
    assert_eq!(settings.daily_wake.max_primary_delta_percent, 3);
    assert_eq!(
        settings.daily_wake.last_run_date.as_deref(),
        Some("2026-05-18")
    );
    assert_eq!(settings.daily_wake.last_run_slots, vec!["2026-05-18#09:15"]);

    save_app_settings_to_path(&settings, config.path()).unwrap();
    let saved = load_app_settings_from_path(config.path()).unwrap();

    assert_eq!(saved.daily_wake, settings.daily_wake);
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

use assert_fs::prelude::*;
use modex_lib::core::quota::{quota_display, snapshot_from_rate_limits};
use modex_lib::core::sync::sync_identity_auth;

#[test]
fn quota_parses_limited_rate_limit_payload() {
    let snapshot = snapshot_from_rate_limits(
        "Team",
        &serde_json::json!({
            "rateLimits": {
                "planType": "team",
                "primary": {
                    "usedPercent": 100,
                    "resetsAt": 1770000000,
                    "windowDurationMins": 300
                },
                "secondary": {
                    "usedPercent": 80,
                    "resetsAt": null,
                    "windowDurationMins": 10080
                },
                "credits": {
                    "hasCredits": false,
                    "unlimited": false
                },
                "rateLimitReachedType": "rate_limit_reached"
            }
        }),
    )
    .unwrap();

    assert!(snapshot.is_limited());
    assert_eq!(snapshot.plan_type.as_deref(), Some("team"));

    let display = quota_display(Some(&snapshot), None);

    assert_eq!(display.status, "limited");
    assert_eq!(display.plan, "团队版");
    assert_eq!(display.primary_percent, 100);
    assert_eq!(display.primary_reset_at, Some(1770000000));
    assert!(display.primary_label.contains("5小时已用 100%"));
    assert_eq!(display.secondary_percent, 80);
    assert_eq!(display.secondary_reset_at, None);
}

#[test]
fn quota_treats_available_credits_as_not_limited() {
    let snapshot = snapshot_from_rate_limits(
        "Team",
        &serde_json::json!({
            "planType": "business",
            "primary": {"usedPercent": 100},
            "credits": {"hasCredits": true, "unlimited": false},
            "rateLimitReachedType": "rate_limit_reached"
        }),
    )
    .unwrap();

    assert!(!snapshot.is_limited());
    assert_eq!(quota_display(Some(&snapshot), None).status, "available");
}

#[test]
fn quota_hides_business_bars_when_no_rate_windows_exist() {
    let snapshot = snapshot_from_rate_limits(
        "Enterprise",
        &serde_json::json!({
            "planType": "business",
            "credits": {"hasCredits": true, "unlimited": true}
        }),
    )
    .unwrap();

    let display = quota_display(Some(&snapshot), None);

    assert_eq!(display.status, "available");
    assert_eq!(display.plan, "企业版");
    assert_eq!(display.primary_label, "");
    assert_eq!(display.primary_percent, 0);
    assert_eq!(display.primary_reset_at, None);
    assert_eq!(display.secondary_label, "");
    assert_eq!(display.secondary_percent, 0);
    assert_eq!(display.secondary_reset_at, None);
}

#[test]
fn quota_team_shows_only_rate_windows_that_are_present() {
    let snapshot = snapshot_from_rate_limits(
        "Team",
        &serde_json::json!({
            "planType": "team",
            "secondary": {"usedPercent": 42, "resetsAt": 1770036000},
            "credits": {"hasCredits": true, "unlimited": false}
        }),
    )
    .unwrap();

    let display = quota_display(Some(&snapshot), None);

    assert_eq!(display.primary_label, "");
    assert_eq!(display.primary_percent, 0);
    assert_eq!(display.secondary_label, "每周已用 42%");
    assert_eq!(display.secondary_percent, 42);
    assert_eq!(display.secondary_reset_at, Some(1770036000));
}

#[test]
fn quota_free_shows_only_weekly_rate_window() {
    let snapshot = snapshot_from_rate_limits(
        "Free",
        &serde_json::json!({
            "planType": "free",
            "primary": {"usedPercent": 10},
            "secondary": {"usedPercent": 55},
            "credits": {"hasCredits": false, "unlimited": false}
        }),
    )
    .unwrap();

    let display = quota_display(Some(&snapshot), None);

    assert_eq!(display.primary_label, "每周已用 55%");
    assert_eq!(display.primary_percent, 55);
    assert_eq!(display.secondary_label, "");
    assert_eq!(display.secondary_percent, 0);
}

#[test]
fn quota_unknown_and_errors_keep_visible_placeholder_labels() {
    let unknown = quota_display(None, None);
    assert_eq!(unknown.primary_label, "5小时已用 -");
    assert_eq!(unknown.secondary_label, "每周已用 -");

    let error = quota_display(None, Some("network unavailable"));
    assert_eq!(error.status, "error");
    assert_eq!(error.primary_label, "5小时已用 -");
    assert_eq!(error.secondary_label, "每周已用 -");
    assert_eq!(error.error.as_deref(), Some("network unavailable"));
}

#[test]
fn switch_identity_replaces_only_source_auth_file() {
    let temp = assert_fs::TempDir::new().unwrap();
    let source = temp.child("source");
    let identity = temp.child("identity");
    source.create_dir_all().unwrap();
    identity.create_dir_all().unwrap();
    source
        .child("auth.json")
        .write_str(r#"{"old": true}"#)
        .unwrap();
    source
        .child("config.toml")
        .write_str("keep = true\n")
        .unwrap();
    identity
        .child("auth.json")
        .write_str(r#"{"new": true}"#)
        .unwrap();

    let target = sync_identity_auth(source.path(), identity.path()).unwrap();

    assert_eq!(target, source.path().join("auth.json"));
    assert_eq!(
        std::fs::read_to_string(source.child("auth.json").path()).unwrap(),
        r#"{"new": true}"#
    );
    assert_eq!(
        std::fs::read_to_string(source.child("config.toml").path()).unwrap(),
        "keep = true\n"
    );
}

use assert_fs::prelude::*;
use modex_lib::core::app_config::DailyWakeSettings;
use modex_lib::core::engine::IdentityView;
use modex_lib::core::quota::QuotaDisplay;
use modex_lib::core::wake::{
    append_wake_log_entry, finalize_wake_quota_evidence, primary_delta_exceeds_limit,
    read_recent_wake_log_entries, should_wake_identity, wake_quota_evidence, WakeAuditEntry,
    WakeDecision, WakeQuotaEvidence, WakeSkipReason, WakeThresholds,
};

#[test]
fn wake_allows_idle_logged_in_team_account_with_weekly_budget() {
    let identity = identity_view(
        "team@example.com",
        "团队版",
        3,
        50,
        true,
        false,
        "available",
    );
    let settings = DailyWakeSettings::default();

    assert_eq!(
        should_wake_identity(&identity, &settings, "2026-05-18"),
        WakeDecision::Wake
    );
}

#[test]
fn wake_skips_when_primary_usage_is_above_threshold() {
    let identity = identity_view(
        "team@example.com",
        "团队版",
        4,
        50,
        true,
        false,
        "available",
    );
    let settings = DailyWakeSettings::default();

    assert_eq!(
        should_wake_identity(&identity, &settings, "2026-05-18"),
        WakeDecision::Skip(WakeSkipReason::PrimaryUsageAboveThreshold)
    );
}

#[test]
fn wake_reports_primary_threshold_before_limited_status() {
    let identity = identity_view(
        "team@example.com",
        "团队版",
        100,
        47,
        true,
        false,
        "limited",
    );
    let settings = DailyWakeSettings::default();

    assert_eq!(
        should_wake_identity(&identity, &settings, "2026-05-18"),
        WakeDecision::Skip(WakeSkipReason::PrimaryUsageAboveThreshold)
    );
}

#[test]
fn wake_skips_when_weekly_remaining_is_below_threshold() {
    let identity = identity_view(
        "team@example.com",
        "团队版",
        1,
        82,
        true,
        false,
        "available",
    );
    let settings = DailyWakeSettings::default();

    assert_eq!(
        should_wake_identity(&identity, &settings, "2026-05-18"),
        WakeDecision::Skip(WakeSkipReason::WeeklyRemainingBelowThreshold)
    );
}

#[test]
fn wake_skips_non_team_and_unknown_quota_accounts() {
    let personal = identity_view(
        "plus@example.com",
        "个人版",
        1,
        10,
        true,
        false,
        "available",
    );
    let unknown = identity_view("team@example.com", "团队版", 0, 0, true, false, "unknown");
    let settings = DailyWakeSettings::default();

    assert_eq!(
        should_wake_identity(&personal, &settings, "2026-05-18"),
        WakeDecision::Skip(WakeSkipReason::NotTeamPlan)
    );
    assert_eq!(
        should_wake_identity(&unknown, &settings, "2026-05-18"),
        WakeDecision::Skip(WakeSkipReason::QuotaUnavailable)
    );
}

#[test]
fn wake_skips_accounts_already_processed_today() {
    let identity = identity_view(
        "team@example.com",
        "团队版",
        1,
        20,
        true,
        false,
        "available",
    );
    let mut settings = DailyWakeSettings::default();
    settings.last_run_date = Some("2026-05-18".to_string());

    assert_eq!(
        should_wake_identity(&identity, &settings, "2026-05-18"),
        WakeDecision::Skip(WakeSkipReason::AlreadyRanToday)
    );
}

#[test]
fn primary_delta_limit_detects_unexpected_consumption() {
    assert!(!primary_delta_exceeds_limit(1, 4, 3));
    assert!(primary_delta_exceeds_limit(1, 5, 3));
    assert!(!primary_delta_exceeds_limit(98, 2, 3));
}

#[test]
fn wake_quota_evidence_confirms_visible_primary_usage() {
    assert_eq!(
        wake_quota_evidence(
            1,
            Some(1_779_000_000),
            2,
            Some(1_779_000_030),
            1_778_982_030
        ),
        WakeQuotaEvidence::Verified("primaryUsageIncreased")
    );
}

#[test]
fn wake_quota_evidence_confirms_stable_active_window() {
    assert_eq!(
        wake_quota_evidence(
            1,
            Some(1_779_000_000),
            1,
            Some(1_779_000_000),
            1_778_982_120
        ),
        WakeQuotaEvidence::Verified("primaryWindowStable")
    );
}

#[test]
fn wake_quota_evidence_rejects_ok_reply_without_window_signal() {
    assert_eq!(
        wake_quota_evidence(
            1,
            Some(1_779_000_000),
            1,
            Some(1_779_000_045),
            1_778_982_045
        ),
        WakeQuotaEvidence::Unverified("primaryWindowMovedWithoutUsage")
    );
    assert_eq!(
        wake_quota_evidence(1, Some(1_779_000_000), 1, None, 1_778_982_045),
        WakeQuotaEvidence::Unverified("missingPrimaryResetAt")
    );
}

#[test]
fn wake_quota_evidence_accepts_settled_backend_update() {
    let initial = WakeQuotaEvidence::Unverified("primaryWindowMovedWithoutUsage");
    let settled = WakeQuotaEvidence::Verified("primaryUsageIncreased");

    assert_eq!(
        finalize_wake_quota_evidence(initial, Some(settled)),
        WakeQuotaEvidence::Verified("primaryUsageIncreased")
    );
}

#[test]
fn wake_audit_log_appends_jsonl_and_reads_newest_first() {
    let temp = assert_fs::TempDir::new().unwrap();
    let log_path = temp.child("wake-runs.jsonl");
    let first = audit_entry("run-1", "skipped");
    let second = audit_entry("run-2", "woke");

    append_wake_log_entry(log_path.path(), &first).unwrap();
    append_wake_log_entry(log_path.path(), &second).unwrap();

    let recent = read_recent_wake_log_entries(log_path.path(), 5).unwrap();

    assert_eq!(recent, vec![second, first]);
}

#[test]
fn wake_audit_log_respects_limit() {
    let temp = assert_fs::TempDir::new().unwrap();
    let log_path = temp.child("wake-runs.jsonl");
    append_wake_log_entry(log_path.path(), &audit_entry("run-1", "skipped")).unwrap();
    append_wake_log_entry(log_path.path(), &audit_entry("run-2", "woke")).unwrap();

    let recent = read_recent_wake_log_entries(log_path.path(), 1).unwrap();

    assert_eq!(recent, vec![audit_entry("run-2", "woke")]);
}

fn identity_view(
    name: &str,
    plan: &str,
    primary_percent: u8,
    weekly_percent: u8,
    logged_in: bool,
    login_expired: bool,
    status: &str,
) -> IdentityView {
    IdentityView {
        name: name.to_string(),
        codex_home: format!("/tmp/{name}"),
        monitor: false,
        workspace_id: None,
        auth_type: Default::default(),
        api_base_url: None,
        logged_in,
        login_expired,
        is_current: false,
        quota: QuotaDisplay {
            status: status.to_string(),
            plan: plan.to_string(),
            primary_label: "5小时已用".to_string(),
            primary_percent,
            primary_reset_at: None,
            secondary_label: "每周已用".to_string(),
            secondary_percent: weekly_percent,
            secondary_reset_at: None,
            credits: "额度可用".to_string(),
            error: None,
        },
    }
}

fn audit_entry(run_id: &str, decision: &str) -> WakeAuditEntry {
    WakeAuditEntry {
        id: format!("{run_id}:team@example.com"),
        run_id: run_id.to_string(),
        timestamp_millis: 1_770_000_000_000,
        level: "info".to_string(),
        source: "dailyWake".to_string(),
        identity_name: Some("team@example.com".to_string()),
        title: "每日唤醒".to_string(),
        message: "team@example.com 已记录唤醒决策".to_string(),
        decision: decision.to_string(),
        reason: None,
        primary_used_percent: Some(1),
        weekly_remaining_percent: Some(80),
        thresholds: WakeThresholds {
            skip_if_primary_used_above_percent: 3,
            skip_if_weekly_remaining_below_percent: 20,
            max_primary_delta_percent: 3,
        },
        detail: serde_json::json!({"templateVersion": 1}),
    }
}

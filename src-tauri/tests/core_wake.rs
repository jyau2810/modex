use assert_fs::prelude::*;
use modex_lib::core::app_config::{AppIdentity, DailyWakeSettings, IdentityAuthType};
use modex_lib::core::engine::IdentityView;
use modex_lib::core::quota::QuotaDisplay;
use modex_lib::core::wake::{
    append_wake_log_entry, finalize_wake_quota_evidence, primary_delta_exceeds_limit,
    read_recent_wake_log_entries, run_wake_prompt, should_wake_identity, wake_quota_evidence,
    WakeAuditEntry, WakeDecision, WakeQuotaEvidence, WakeSkipReason, WakeThresholds,
};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{fs, time::Duration};

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
fn wake_quota_evidence_confirms_fresh_window_move_without_percent_change() {
    assert_eq!(
        wake_quota_evidence(
            1,
            Some(1_779_000_000),
            1,
            Some(1_779_000_045),
            1_778_982_045
        ),
        WakeQuotaEvidence::Verified("primaryWindowAdvanced")
    );
}

#[test]
fn wake_quota_evidence_rejects_ok_reply_without_window_signal() {
    assert_eq!(
        wake_quota_evidence(
            1,
            Some(1_779_000_000),
            1,
            Some(1_779_060_000),
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

#[cfg(unix)]
#[test]
fn wake_prompt_starts_thread_turn_without_archiving_session() {
    let temp = assert_fs::TempDir::new().unwrap();
    let requests_path = temp.child("requests.jsonl");
    let script_path = temp.child("fake-codex.sh");
    script_path
        .write_str(&format!(
            "#!/bin/sh
requests_path='{}'
while IFS= read -r line; do
  printf '%s\n' \"$line\" >> \"$requests_path\"
  case \"$line\" in
    *'\"method\":\"initialize\"'*)
      printf '%s\n' '{{\"id\":1,\"result\":{{\"serverInfo\":{{\"name\":\"fake-codex\"}}}}}}'
      ;;
    *'\"method\":\"thread/start\"'*)
      printf '%s\n' '{{\"id\":2,\"result\":{{\"thread\":{{\"id\":\"thread-1\"}}}}}}'
      ;;
    *'\"method\":\"turn/start\"'*)
      printf '%s\n' '{{\"id\":3,\"result\":{{\"turn\":{{\"id\":\"turn-1\"}}}}}}'
      printf '%s\n' '{{\"method\":\"item/completed\",\"params\":{{\"threadId\":\"thread-1\",\"turnId\":\"turn-1\",\"item\":{{\"type\":\"agentMessage\",\"text\":\"OK\"}}}}}}'
      printf '%s\n' '{{\"method\":\"turn/completed\",\"params\":{{\"threadId\":\"thread-1\",\"turn\":{{\"id\":\"turn-1\",\"status\":\"completed\"}}}}}}'
      ;;
    *'\"method\":\"thread/archive\"'*)
      printf '%s\n' '{{\"id\":4,\"result\":{{}}}}'
      ;;
  esac
done
",
            requests_path.path().display()
        ))
        .unwrap();
    fs::set_permissions(script_path.path(), fs::Permissions::from_mode(0o755)).unwrap();

    let identity_home = temp.child("identity-home");
    identity_home.create_dir_all().unwrap();
    let result = run_wake_prompt(
        script_path.path().to_str().unwrap(),
        &AppIdentity {
            name: "team@example.com".to_string(),
            codex_home: identity_home.path().to_path_buf(),
            monitor: false,
            workspace_id: None,
            auth_type: IdentityAuthType::ChatGpt,
            api_base_url: None,
        },
        "Good morning",
        Duration::from_secs(2),
    )
    .unwrap();

    let request_methods = fs::read_to_string(requests_path.path())
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();

    assert_eq!(result.exit_code, Some(0));
    assert!(!result.timed_out);
    assert_eq!(result.thread_id.as_deref(), Some("thread-1"));
    assert_eq!(result.last_message, "OK");
    assert_eq!(
        request_methods
            .iter()
            .map(|request| request["method"].as_str().unwrap().to_string())
            .collect::<Vec<_>>(),
        vec![
            "initialize".to_string(),
            "thread/start".to_string(),
            "turn/start".to_string(),
        ]
    );

    let thread_request = request_methods
        .iter()
        .find(|request| request["method"] == "thread/start")
        .unwrap();
    assert_eq!(thread_request["params"]["approvalPolicy"], "never");
    assert_eq!(thread_request["params"]["sandbox"], "read-only");
    assert_eq!(thread_request["params"]["threadSource"], "user");
    assert!(thread_request["params"].get("sessionStartSource").is_none());
    assert!(thread_request["params"].get("baseInstructions").is_none());
    assert!(thread_request["params"].get("developerInstructions").is_none());

    let turn_request = request_methods
        .iter()
        .find(|request| request["method"] == "turn/start")
        .unwrap();
    assert_eq!(turn_request["params"]["threadId"], "thread-1");
    assert_eq!(turn_request["params"]["sandboxPolicy"]["type"], "readOnly");
    assert_eq!(turn_request["params"]["sandboxPolicy"]["networkAccess"], false);
    assert!(turn_request["params"].get("approvalPolicy").is_none());
    assert!(turn_request["params"].get("effort").is_none());
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

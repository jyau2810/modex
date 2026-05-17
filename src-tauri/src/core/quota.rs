use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::auth::plan_label;
use super::{ModexError, ModexResult};

const LIMITED_REACHED_TYPES: &[&str] = &[
    "rate_limit_reached",
    "workspace_owner_credits_depleted",
    "workspace_member_credits_depleted",
    "workspace_owner_usage_limit_reached",
    "workspace_member_usage_limit_reached",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateWindow {
    pub used_percent: u8,
    pub resets_at: Option<i64>,
    pub window_duration_mins: Option<i64>,
}

impl RateWindow {
    pub fn is_limited(&self) -> bool {
        self.used_percent >= 100
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaSnapshot {
    pub identity: String,
    pub plan_type: Option<String>,
    pub primary: Option<RateWindow>,
    pub secondary: Option<RateWindow>,
    pub credits_has_credits: Option<bool>,
    pub credits_unlimited: Option<bool>,
    pub reached_type: Option<String>,
}

impl QuotaSnapshot {
    pub fn has_usable_credits(&self) -> bool {
        self.credits_unlimited == Some(true) || self.credits_has_credits == Some(true)
    }

    pub fn is_limited(&self) -> bool {
        if self.has_usable_credits() {
            return false;
        }
        let reached = self
            .reached_type
            .as_deref()
            .is_some_and(|value| LIMITED_REACHED_TYPES.contains(&value));
        let window_limited = [&self.primary, &self.secondary]
            .into_iter()
            .flatten()
            .any(RateWindow::is_limited);
        reached || window_limited
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaDisplay {
    pub status: String,
    pub plan: String,
    pub primary_label: String,
    pub primary_percent: u8,
    pub primary_reset_at: Option<i64>,
    pub secondary_label: String,
    pub secondary_percent: u8,
    pub secondary_reset_at: Option<i64>,
    pub credits: String,
    pub error: Option<String>,
}

pub fn snapshot_from_rate_limits(identity: &str, payload: &Value) -> ModexResult<QuotaSnapshot> {
    let rate_limits = payload.get("rateLimits").unwrap_or(payload);
    if !rate_limits.is_object() {
        return Err(ModexError::from("rateLimits returned non-object result"));
    }
    let credits = rate_limits.get("credits").unwrap_or(&Value::Null);
    Ok(QuotaSnapshot {
        identity: identity.to_string(),
        plan_type: rate_limits
            .get("planType")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        primary: window(rate_limits.get("primary")),
        secondary: window(rate_limits.get("secondary")),
        credits_has_credits: credits.get("hasCredits").and_then(Value::as_bool),
        credits_unlimited: credits.get("unlimited").and_then(Value::as_bool),
        reached_type: rate_limits
            .get("rateLimitReachedType")
            .and_then(Value::as_str)
            .map(ToString::to_string),
    })
}

pub fn quota_display(snapshot: Option<&QuotaSnapshot>, error: Option<&str>) -> QuotaDisplay {
    if let Some(error) = error {
        return QuotaDisplay {
            status: "error".to_string(),
            plan: "计划未知".to_string(),
            primary_label: "5小时已用 -".to_string(),
            primary_percent: 0,
            primary_reset_at: None,
            secondary_label: "每周已用 -".to_string(),
            secondary_percent: 0,
            secondary_reset_at: None,
            credits: "额度未知".to_string(),
            error: Some(error.to_string()),
        };
    }
    let Some(snapshot) = snapshot else {
        return QuotaDisplay {
            status: "unknown".to_string(),
            plan: "计划未知".to_string(),
            primary_label: "5小时已用 -".to_string(),
            primary_percent: 0,
            primary_reset_at: None,
            secondary_label: "每周已用 -".to_string(),
            secondary_percent: 0,
            secondary_reset_at: None,
            credits: "额度未知".to_string(),
            error: None,
        };
    };
    let plan = plan_label(snapshot.plan_type.as_deref());
    let is_free = snapshot
        .plan_type
        .as_deref()
        .is_some_and(|plan| plan.eq_ignore_ascii_case("free"));
    let (
        primary_label,
        primary_percent,
        primary_reset_at,
        secondary_label,
        secondary_percent,
        secondary_reset_at,
    ) = if is_free {
        let (label, percent, reset_at) = window_display(
            "每周",
            snapshot.secondary.as_ref().or(snapshot.primary.as_ref()),
        );
        (label, percent, reset_at, String::new(), 0, None)
    } else {
        let (primary_label, primary_percent, primary_reset_at) =
            window_display("5小时", snapshot.primary.as_ref());
        let (secondary_label, secondary_percent, secondary_reset_at) =
            window_display("每周", snapshot.secondary.as_ref());
        (
            primary_label,
            primary_percent,
            primary_reset_at,
            secondary_label,
            secondary_percent,
            secondary_reset_at,
        )
    };
    QuotaDisplay {
        status: if snapshot.is_limited() {
            "limited"
        } else {
            "available"
        }
        .to_string(),
        plan,
        primary_label,
        primary_percent,
        primary_reset_at,
        secondary_label,
        secondary_percent,
        secondary_reset_at,
        credits: credits_label(snapshot),
        error: None,
    }
}

fn window(raw: Option<&Value>) -> Option<RateWindow> {
    let raw = raw?.as_object()?;
    Some(RateWindow {
        used_percent: raw
            .get("usedPercent")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .min(100) as u8,
        resets_at: raw.get("resetsAt").and_then(Value::as_i64),
        window_duration_mins: raw.get("windowDurationMins").and_then(Value::as_i64),
    })
}

fn window_display(title: &str, window: Option<&RateWindow>) -> (String, u8, Option<i64>) {
    let Some(window) = window else {
        return (String::new(), 0, None);
    };
    (
        format!("{title}已用 {}%", window.used_percent),
        window.used_percent,
        window.resets_at,
    )
}

fn credits_label(snapshot: &QuotaSnapshot) -> String {
    if snapshot.credits_unlimited == Some(true) {
        "额度无限".to_string()
    } else if snapshot.credits_has_credits == Some(true) {
        "额度可用".to_string()
    } else if snapshot.credits_has_credits == Some(false) {
        "无额外额度".to_string()
    } else {
        "额度未知".to_string()
    }
}

use std::collections::HashMap;

use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::core::engine::IdentityView;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationSpec {
    pub title: String,
    pub body: String,
}

pub fn switch_success_notification(name: &str) -> NotificationSpec {
    NotificationSpec {
        title: "账号切换成功".to_string(),
        body: format!("已切换到账号：{name}"),
    }
}

pub fn refresh_notifications(
    before: &[IdentityView],
    after: &[IdentityView],
) -> Vec<NotificationSpec> {
    let before_by_name = before
        .iter()
        .map(|identity| (identity.name.as_str(), identity))
        .collect::<HashMap<_, _>>();
    let mut notifications = Vec::new();
    for identity in after {
        let previous = before_by_name.get(identity.name.as_str()).copied();
        if became_login_expired(previous, identity) {
            notifications.push(NotificationSpec {
                title: "登录失效".to_string(),
                body: format!("{} 需要重新登录。", identity.name),
            });
            continue;
        }
        if refresh_error_changed(previous, identity) {
            notifications.push(NotificationSpec {
                title: "刷新失败".to_string(),
                body: format!(
                    "{} 刷新失败：{}",
                    identity.name,
                    identity.quota.error.as_deref().unwrap_or("未知错误")
                ),
            });
            continue;
        }
        if quota_recovered(previous, identity) {
            notifications.push(NotificationSpec {
                title: "额度已恢复".to_string(),
                body: format!("{} 额度已恢复，可以继续使用。", identity.name),
            });
        }
    }
    notifications
}

pub fn send_notification(app: &AppHandle, notification: &NotificationSpec) {
    if let Err(error) = app
        .notification()
        .builder()
        .title(&notification.title)
        .body(&notification.body)
        .show()
    {
        eprintln!("modex notification failed: {error}");
    }
}

pub fn send_notifications(app: &AppHandle, notifications: &[NotificationSpec]) {
    for notification in notifications {
        send_notification(app, notification);
    }
}

fn became_login_expired(previous: Option<&IdentityView>, current: &IdentityView) -> bool {
    current.login_expired && !previous.is_some_and(|identity| identity.login_expired)
}

fn refresh_error_changed(previous: Option<&IdentityView>, current: &IdentityView) -> bool {
    if current.login_expired || current.quota.status != "error" {
        return false;
    }
    previous
        .map(|identity| {
            identity.quota.status != "error" || identity.quota.error != current.quota.error
        })
        .unwrap_or(true)
}

fn quota_recovered(previous: Option<&IdentityView>, current: &IdentityView) -> bool {
    previous.is_some_and(|identity| identity.quota.status == "limited")
        && current.quota.status == "available"
}

#[cfg(test)]
mod tests {
    use crate::core::engine::IdentityView;
    use crate::core::quota::QuotaDisplay;

    use super::*;

    #[test]
    fn plans_switch_success_notification() {
        assert_eq!(
            switch_success_notification("team@example.com"),
            NotificationSpec {
                title: "账号切换成功".to_string(),
                body: "已切换到账号：team@example.com".to_string(),
            }
        );
    }

    #[test]
    fn plans_login_expired_and_refresh_failed_notifications() {
        let before = vec![
            identity("expired@example.com", "available", None, true, false),
            identity("broken@example.com", "available", None, true, false),
        ];
        let after = vec![
            identity(
                "expired@example.com",
                "error",
                Some("登录已过期"),
                false,
                true,
            ),
            identity(
                "broken@example.com",
                "error",
                Some("timed out waiting for account/rateLimits/read"),
                true,
                false,
            ),
        ];

        assert_eq!(
            refresh_notifications(&before, &after),
            vec![
                NotificationSpec {
                    title: "登录失效".to_string(),
                    body: "expired@example.com 需要重新登录。".to_string(),
                },
                NotificationSpec {
                    title: "刷新失败".to_string(),
                    body:
                        "broken@example.com 刷新失败：timed out waiting for account/rateLimits/read"
                            .to_string(),
                },
            ]
        );
    }

    #[test]
    fn plans_quota_recovered_only_when_limited_becomes_available() {
        let before = vec![
            identity("recovered@example.com", "limited", None, true, false),
            identity("still-ok@example.com", "available", None, true, false),
        ];
        let after = vec![
            identity("recovered@example.com", "available", None, true, false),
            identity("still-ok@example.com", "available", None, true, false),
        ];

        assert_eq!(
            refresh_notifications(&before, &after),
            vec![NotificationSpec {
                title: "额度已恢复".to_string(),
                body: "recovered@example.com 额度已恢复，可以继续使用。".to_string(),
            }]
        );
    }

    fn identity(
        name: &str,
        quota_status: &str,
        error: Option<&str>,
        logged_in: bool,
        login_expired: bool,
    ) -> IdentityView {
        IdentityView {
            name: name.to_string(),
            codex_home: format!("/tmp/{name}"),
            monitor: true,
            workspace_id: None,
            logged_in,
            login_expired,
            is_current: false,
            quota: QuotaDisplay {
                status: quota_status.to_string(),
                plan: "团队版".to_string(),
                primary_label: String::new(),
                primary_percent: 0,
                primary_reset_at: None,
                secondary_label: String::new(),
                secondary_percent: 0,
                secondary_reset_at: None,
                credits: String::new(),
                error: error.map(ToString::to_string),
            },
        }
    }
}

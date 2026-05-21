use std::collections::HashMap;
#[cfg(target_os = "macos")]
use std::ffi::{c_char, c_int, CStr, CString};
use std::fs::OpenOptions;
use std::io::Write;
#[cfg(target_os = "macos")]
use std::path::Path;

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

pub fn switch_failure_notification(name: &str, reason: &str) -> NotificationSpec {
    NotificationSpec {
        title: "账号切换失败".to_string(),
        body: format!("{name} 切换失败：{reason}"),
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
    log_notification_attempt(notification, "attempt");

    #[cfg(target_os = "macos")]
    {
        if !current_process_supports_macos_notifications() {
            log_notification_attempt(notification, "skipped: unbundled process");
            return;
        }
        log_notification_permission(app, notification);
        match send_macos_foreground_notification(notification) {
            Ok(status) => log_notification_attempt(notification, status),
            Err(error) => {
                log_notification_attempt(notification, &format!("failed: {error}"));
                eprintln!("modex notification failed: {error}");
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        log_notification_permission(app, notification);
        send_tauri_notification(app, notification);
    }
}

fn log_notification_permission(app: &AppHandle, notification: &NotificationSpec) {
    match app.notification().permission_state() {
        Ok(permission) => {
            log_notification_attempt(notification, &format!("permission: {permission:?}"))
        }
        Err(error) => log_notification_attempt(notification, &format!("permission_error: {error}")),
    }
}

#[cfg(not(target_os = "macos"))]
fn send_tauri_notification(app: &AppHandle, notification: &NotificationSpec) {
    match app
        .notification()
        .builder()
        .title(&notification.title)
        .body(&notification.body)
        .show()
    {
        Ok(()) => log_notification_attempt(notification, "submitted"),
        Err(error) => {
            log_notification_attempt(notification, &format!("failed: {error}"));
            eprintln!("modex notification failed: {error}");
        }
    }
}

#[cfg(target_os = "macos")]
fn send_macos_foreground_notification(
    notification: &NotificationSpec,
) -> Result<&'static str, String> {
    let title = cstring_field(&notification.title)?;
    let body = cstring_field(&notification.body)?;
    let status = unsafe { modex_send_user_notification(title.as_ptr(), body.as_ptr()) };
    match status {
        1 => Ok("delivered: usernotifications"),
        2 => Ok("delivered: nsusernotification"),
        -1 => Err("macOS notification bridge rejected the title or body".to_string()),
        -2 => Err(macos_notification_error(
            "UserNotifications authorization denied",
        )),
        -3 => Err(macos_notification_error(
            "UserNotifications authorization failed",
        )),
        -4 => Err(macos_notification_error(
            "UserNotifications add request failed",
        )),
        -5 => Err(macos_notification_error(
            "UserNotifications add request timed out",
        )),
        _ => Err(macos_notification_error(&format!(
            "macOS notification bridge failed with status {status}"
        ))),
    }
}

#[cfg(target_os = "macos")]
fn current_process_supports_macos_notifications() -> bool {
    std::env::current_exe()
        .ok()
        .as_deref()
        .is_some_and(is_bundled_macos_executable)
}

#[cfg(target_os = "macos")]
fn is_bundled_macos_executable(path: &Path) -> bool {
    let Some(macos_dir) = path.parent() else {
        return false;
    };
    if !path_name_eq(macos_dir.file_name(), "MacOS") {
        return false;
    }

    let Some(contents_dir) = macos_dir.parent() else {
        return false;
    };
    if !path_name_eq(contents_dir.file_name(), "Contents") {
        return false;
    }

    contents_dir
        .parent()
        .and_then(Path::extension)
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("app"))
}

#[cfg(target_os = "macos")]
fn path_name_eq(value: Option<&std::ffi::OsStr>, expected: &str) -> bool {
    value.and_then(|value| value.to_str()) == Some(expected)
}

#[cfg(target_os = "macos")]
fn cstring_field(value: &str) -> Result<CString, String> {
    CString::new(value).map_err(|_| "notification text contains a NUL byte".to_string())
}

#[cfg(target_os = "macos")]
fn macos_notification_error(message: &str) -> String {
    match macos_last_notification_error() {
        Some(detail) => format!("{message}: {detail}"),
        None => message.to_string(),
    }
}

#[cfg(target_os = "macos")]
fn macos_last_notification_error() -> Option<String> {
    let ptr = unsafe { modex_last_notification_error() };
    if ptr.is_null() {
        return None;
    }
    let value = unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned();
    (!value.is_empty()).then_some(value)
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn modex_send_user_notification(title: *const c_char, body: *const c_char) -> c_int;
    fn modex_last_notification_error() -> *const c_char;
}

fn log_notification_attempt(notification: &NotificationSpec, status: &str) {
    let Some(mut path) = dirs::data_dir() else {
        return;
    };
    path.push("Modex");
    if std::fs::create_dir_all(&path).is_err() {
        return;
    }
    path.push("notifications.log");
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let _ = writeln!(
        file,
        "{}\t{}\t{}\t{}",
        timestamp_millis(),
        status,
        notification.title.replace('\t', " "),
        notification.body.replace('\t', " ")
    );
}

fn timestamp_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
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
    #[cfg(target_os = "macos")]
    use std::path::Path;

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
    fn plans_switch_failure_notification() {
        assert_eq!(
            switch_failure_notification("team@example.com", "Codex 未退出，账号切换已取消。"),
            NotificationSpec {
                title: "账号切换失败".to_string(),
                body: "team@example.com 切换失败：Codex 未退出，账号切换已取消。".to_string(),
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

    #[cfg(target_os = "macos")]
    #[test]
    fn detects_bundled_macos_executable_paths() {
        assert!(is_bundled_macos_executable(Path::new(
            "/Applications/Modex.app/Contents/MacOS/modex",
        )));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn rejects_unbundled_macos_executable_paths() {
        assert!(!is_bundled_macos_executable(Path::new(
            "/Users/dev/modex/src-tauri/target/debug/modex",
        )));
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
            auth_type: Default::default(),
            api_base_url: None,
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

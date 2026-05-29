#[cfg(target_os = "macos")]
use objc2_app_kit::NSStatusItemBehavior;
#[cfg(target_os = "macos")]
use objc2_foundation::NSString;
#[cfg(target_os = "macos")]
use objc2_foundation::NSUserDefaults;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{image::Image, AppHandle, Emitter, Manager};

use crate::commands::{show_main_window, start_refresh_all_with_events, ModexState};
use crate::core::app_config::IdentityAuthType;

const TRAY_ID: &str = "modex-main";
#[cfg(target_os = "macos")]
const MACOS_TRAY_AUTOSAVE_NAME: &str = "local.modex.statusItem";
#[cfg(target_os = "macos")]
const MACOS_TRAY_VISIBLE_PREF_PREFIX: &str = "NSStatusItem VisibleCC ";
#[cfg(target_os = "macos")]
const MACOS_TRAY_LEGACY_VISIBLE_PREF_KEY: &str = "NSStatusItem VisibleCC Item-0";
const TRAY_ICON_BYTES: &[u8] = include_bytes!("../icons/tray-icon.png");
const MENU_OPEN: &str = "open";
const MENU_SETTINGS: &str = "settings";
const MENU_REFRESH: &str = "refresh";
const MENU_QUIT: &str = "quit";
const IDENTITY_PREFIX: &str = "identity::";
const LOGIN_PREFIX: &str = "login::";

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_menu(app)?;
    #[cfg(target_os = "macos")]
    seed_macos_tray_visibility_preference();
    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip("Modex")
        .show_menu_on_left_click(true)
        .on_menu_event(handle_menu_event);
    tray = tray.icon(Image::from_bytes(TRAY_ICON_BYTES)?);
    #[cfg(target_os = "macos")]
    {
        tray = tray.icon_as_template(true);
    }
    let tray = tray.build(app)?;
    #[cfg(target_os = "macos")]
    configure_macos_tray_status_item(&tray)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn macos_tray_autosave_name() -> &'static str {
    MACOS_TRAY_AUTOSAVE_NAME
}

#[cfg(target_os = "macos")]
fn macos_tray_visibility_preference_key() -> String {
    format!(
        "{MACOS_TRAY_VISIBLE_PREF_PREFIX}{}",
        macos_tray_autosave_name()
    )
}

#[cfg(target_os = "macos")]
fn seed_macos_tray_visibility_preference() {
    let defaults = NSUserDefaults::standardUserDefaults();
    let key = NSString::from_str(&macos_tray_visibility_preference_key());
    let legacy_key = NSString::from_str(MACOS_TRAY_LEGACY_VISIBLE_PREF_KEY);
    defaults.removeObjectForKey(&legacy_key);
    if defaults.objectForKey(&key).is_none() {
        defaults.setBool_forKey(true, &key);
    }
}

#[cfg(target_os = "macos")]
fn macos_tray_visibility_preference() -> bool {
    let defaults = NSUserDefaults::standardUserDefaults();
    let key = NSString::from_str(&macos_tray_visibility_preference_key());
    defaults.boolForKey(&key)
}

#[cfg(target_os = "macos")]
fn macos_tray_allows_user_removal() -> bool {
    true
}

#[cfg(target_os = "macos")]
fn configure_macos_tray_status_item(tray: &tauri::tray::TrayIcon<tauri::Wry>) -> tauri::Result<()> {
    tray.with_inner_tray_icon(|inner| {
        if let Some(status_item) = inner.ns_status_item() {
            let autosave_name = NSString::from_str(macos_tray_autosave_name());
            status_item.setAutosaveName(Some(&autosave_name));
            if macos_tray_allows_user_removal() {
                status_item
                    .setBehavior(status_item.behavior() | NSStatusItemBehavior::RemovalAllowed);
            }
            status_item.setVisible(macos_tray_visibility_preference());
        }
    })?;
    Ok(())
}

pub fn refresh_menu(app: &AppHandle) -> tauri::Result<()> {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let menu = build_menu(app)?;
        tray.set_menu(Some(menu))?;
    }
    Ok(())
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::new(app)?;
    let state = app.state::<ModexState>();
    let refreshing = state.is_refreshing();
    let app_state = state
        .engine
        .lock()
        .expect("Modex state lock poisoned")
        .app_state();

    let current = app_state
        .current_identity_name
        .as_deref()
        .unwrap_or("未选择账号");
    let status = MenuItem::with_id(
        app,
        "status",
        format!("Modex: {current}"),
        false,
        None::<&str>,
    )?;
    menu.append(&status)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;

    if app_state.identities.is_empty() {
        let empty = MenuItem::with_id(app, "empty", "暂无账号", false, None::<&str>)?;
        menu.append(&empty)?;
    } else {
        for identity in app_state.identities {
            let available = identity_menu_available(
                identity.logged_in,
                identity.login_expired,
                identity.quota.status.as_str(),
            );
            let item = CheckMenuItem::with_id(
                app,
                format!("{IDENTITY_PREFIX}{}", identity.name),
                identity_menu_label(
                    &identity.name,
                    identity.logged_in,
                    identity.login_expired,
                    identity.quota.status.as_str(),
                ),
                identity_menu_selectable(available, identity.is_current),
                identity.is_current,
                None::<&str>,
            )?;
            menu.append(&item)?;
            if identity_menu_relogin_available(
                &identity.auth_type,
                identity.logged_in,
                identity.login_expired,
            ) {
                let login_item = MenuItem::with_id(
                    app,
                    format!("{LOGIN_PREFIX}{}", identity.name),
                    format!("重新登录 {}", identity.name),
                    !refreshing,
                    None::<&str>,
                )?;
                menu.append(&login_item)?;
            }
        }
    }

    menu.append(&PredefinedMenuItem::separator(app)?)?;
    let refresh_label = if refreshing {
        "正在刷新配额..."
    } else {
        "刷新配额"
    };
    menu.append(&MenuItem::with_id(
        app,
        MENU_REFRESH,
        refresh_label,
        !refreshing,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        MENU_OPEN,
        "打开 Modex",
        true,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        MENU_SETTINGS,
        "设置",
        true,
        None::<&str>,
    )?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(
        app,
        MENU_QUIT,
        "退出",
        true,
        None::<&str>,
    )?)?;
    Ok(menu)
}

fn handle_menu_event(app: &AppHandle, event: tauri::menu::MenuEvent) {
    let id = event.id().as_ref().to_string();
    match id.as_str() {
        MENU_OPEN => show_main_window(app),
        MENU_SETTINGS => {
            show_main_window(app);
            let _ = app.emit("modex://open-settings", ());
        }
        MENU_REFRESH => {
            start_refresh_all_with_events(app.clone(), "tray");
        }
        MENU_QUIT => app.exit(0),
        _ if id.starts_with(IDENTITY_PREFIX) => {
            let name = id.trim_start_matches(IDENTITY_PREFIX);
            let state = app.state::<ModexState>();
            if let Err(error) =
                crate::commands::switch_identity_with_notifications(app, &state, name)
            {
                eprintln!("modex tray switch failed: {error}");
            }
            let _ = refresh_menu(app);
            crate::commands::emit_state_updated(app);
        }
        _ if id.starts_with(LOGIN_PREFIX) => {
            let name = id.trim_start_matches(LOGIN_PREFIX).to_string();
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = crate::commands::login_identity(app.clone(), name).await {
                    eprintln!("modex tray login failed: {error}");
                }
                let _ = refresh_menu(&app);
                crate::commands::emit_state_updated(&app);
            });
        }
        _ => {}
    }
}

fn identity_menu_label(
    name: &str,
    logged_in: bool,
    login_expired: bool,
    quota_status: &str,
) -> String {
    let available = identity_menu_available(logged_in, login_expired, quota_status);
    let marker = if available { "🟢" } else { "🔴" };
    let suffix = if login_expired {
        "（需重新登录）"
    } else if !logged_in {
        "（未登录）"
    } else if quota_status == "limited" {
        "（配额受限）"
    } else {
        ""
    };
    format!("{marker} {name}{suffix}")
}

fn identity_menu_available(logged_in: bool, login_expired: bool, quota_status: &str) -> bool {
    logged_in && !login_expired && quota_status != "limited"
}

fn identity_menu_selectable(available: bool, is_current: bool) -> bool {
    available && !is_current
}

fn identity_menu_relogin_available(
    auth_type: &IdentityAuthType,
    logged_in: bool,
    login_expired: bool,
) -> bool {
    auth_type == &IdentityAuthType::ChatGpt && (!logged_in || login_expired)
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    use super::macos_tray_allows_user_removal;
    #[cfg(target_os = "macos")]
    use super::macos_tray_autosave_name;
    #[cfg(target_os = "macos")]
    use super::macos_tray_visibility_preference_key;
    use crate::core::app_config::IdentityAuthType;

    use super::{
        identity_menu_available, identity_menu_label, identity_menu_relogin_available,
        identity_menu_selectable,
    };

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_tray_autosave_name_is_unique_to_modex() {
        assert_eq!(macos_tray_autosave_name(), "local.modex.statusItem");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_tray_visibility_preference_uses_unique_autosave_name() {
        assert_eq!(
            macos_tray_visibility_preference_key(),
            "NSStatusItem VisibleCC local.modex.statusItem"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_tray_status_item_is_user_removable() {
        assert!(macos_tray_allows_user_removal());
    }

    #[test]
    fn identity_menu_label_adds_emoji_status_dot() {
        assert_eq!(
            identity_menu_label("team@example.com", true, false, "available"),
            "🟢 team@example.com"
        );
        assert_eq!(
            identity_menu_label("limited@example.com", true, false, "limited"),
            "🔴 limited@example.com（配额受限）"
        );
        assert_eq!(
            identity_menu_label("backup@example.com", false, false, "available"),
            "🔴 backup@example.com（未登录）"
        );
        assert_eq!(
            identity_menu_label("expired@example.com", true, true, "available"),
            "🔴 expired@example.com（需重新登录）"
        );
    }

    #[test]
    fn identity_menu_availability_treats_limited_quota_as_unavailable() {
        assert!(identity_menu_available(true, false, "available"));
        assert!(identity_menu_available(true, false, "unknown"));
        assert!(!identity_menu_available(true, false, "limited"));
        assert!(!identity_menu_available(false, false, "available"));
        assert!(!identity_menu_available(true, true, "available"));
    }

    #[test]
    fn identity_menu_selectable_disables_current_or_unavailable_accounts() {
        assert!(identity_menu_selectable(true, false));
        assert!(!identity_menu_selectable(true, true));
        assert!(!identity_menu_selectable(false, false));
    }

    #[test]
    fn identity_menu_offers_relogin_for_unavailable_chatgpt_accounts() {
        assert!(identity_menu_relogin_available(
            &IdentityAuthType::ChatGpt,
            false,
            false
        ));
        assert!(identity_menu_relogin_available(
            &IdentityAuthType::ChatGpt,
            true,
            true
        ));
        assert!(!identity_menu_relogin_available(
            &IdentityAuthType::ChatGpt,
            true,
            false
        ));
        assert!(!identity_menu_relogin_available(
            &IdentityAuthType::ApiKey,
            false,
            true
        ));
    }
}

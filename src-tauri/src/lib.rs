mod commands;
pub mod core;
mod tray;

use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            commands::get_app_state,
            commands::add_identity,
            commands::delete_identity,
            commands::switch_identity,
            commands::login_identity,
            commands::refresh_identity,
            commands::refresh_all,
            commands::update_settings,
            commands::open_identity_directory,
            commands::open_main_window,
        ])
        .setup(|app| {
            let engine = core::engine::AppEngine::load()?;
            app.manage(commands::ModexState::new(engine));
            #[cfg(target_os = "macos")]
            {
                let starts_visible = app
                    .get_webview_window("main")
                    .and_then(|window| window.is_visible().ok())
                    .unwrap_or(false);
                if starts_visible {
                    commands::show_main_window(app.app_handle());
                } else {
                    app.set_activation_policy(tauri::ActivationPolicy::Accessory);
                }
            }
            {
                use tauri::WindowEvent;
                if let Some(window) = app.get_webview_window("main") {
                    let window_for_handler = window.clone();
                    #[cfg(target_os = "macos")]
                    let app_handle = app.handle().clone();
                    window.on_window_event(move |event| {
                        if let WindowEvent::CloseRequested { api, .. } = event {
                            api.prevent_close();
                            let _ = window_for_handler.hide();
                            #[cfg(target_os = "macos")]
                            {
                                let _ = app_handle
                                    .set_activation_policy(tauri::ActivationPolicy::Accessory);
                            }
                        }
                    });
                }
            }
            tray::setup(app.app_handle())?;
            commands::start_startup_refresh(app.app_handle().clone());
            commands::start_background_monitor(app.app_handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Modex");
}

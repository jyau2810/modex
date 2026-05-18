use assert_fs::prelude::*;
use modex_lib::core::app_config::AppSettings;
use modex_lib::core::codex::{open_codex_app_launch_command, resolve_codex_binary_with};

#[cfg(target_os = "macos")]
use modex_lib::core::codex::macos_quit_codex_app_script;

#[test]
fn resolves_codex_to_app_cli_when_path_lookup_fails() {
    let temp = assert_fs::TempDir::new().unwrap();
    let app_cli = temp.child("Codex.app/Contents/Resources/codex");
    app_cli.touch().unwrap();

    let resolved = resolve_codex_binary_with("codex", |_| None, &[app_cli.path().to_path_buf()]);

    assert_eq!(resolved, app_cli.path().to_path_buf());
}

#[test]
fn resolves_configured_path_without_path_lookup() {
    let resolved = resolve_codex_binary_with("~/bin/codex", |_| Some("/wrong/codex".into()), &[]);

    assert!(resolved.ends_with("bin/codex"));
}

#[cfg(target_os = "macos")]
#[test]
fn macos_switch_launches_codex_app_without_creating_a_project() {
    let temp = assert_fs::TempDir::new().unwrap();
    let settings = AppSettings::default_for_home(temp.path().to_path_buf());

    let command = open_codex_app_launch_command(&settings);

    assert_eq!(command.program.to_string_lossy(), "open");
    assert_eq!(command.args, vec!["-a".to_string(), "Codex".to_string()]);
    assert!(command.envs.is_empty());
}

#[cfg(target_os = "macos")]
#[test]
fn macos_quit_script_waits_until_codex_has_stopped_before_reopening() {
    let script = macos_quit_codex_app_script("Codex");

    assert!(script.contains(r#"if application "Codex" is running then"#));
    assert!(script.contains(r#"tell application "Codex" to quit"#));
    assert!(script.contains("repeat with _attempt in 1 to 50"));
    assert!(script.contains(r#"if application "Codex" is not running then exit repeat"#));
    assert!(script.contains("delay 0.1"));
}

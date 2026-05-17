use assert_fs::prelude::*;
use modex_lib::core::app_config::AppSettings;
use modex_lib::core::codex::{open_codex_app_launch_command, resolve_codex_binary_with};

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

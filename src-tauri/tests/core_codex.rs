use assert_fs::prelude::*;
use modex_lib::core::app_config::{AppIdentity, AppSettings, IdentityAuthType};
use modex_lib::core::codex::{
    api_key_login_invocation, open_codex_app_launch_command, resolve_codex_binary_with,
    ProgramInvocation,
};

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

#[test]
fn api_key_login_command_reads_key_from_stdin() {
    let settings = AppSettings::default_for_home("/tmp/modex-test".into());
    let identity = AppIdentity {
        name: "API".to_string(),
        codex_home: "/tmp/modex-test/.modex/api".into(),
        monitor: false,
        workspace_id: None,
        auth_type: IdentityAuthType::ApiKey,
        api_base_url: None,
    };

    let invocation: ProgramInvocation = api_key_login_invocation(&settings, &identity);

    assert_eq!(
        invocation.args,
        vec!["login".to_string(), "--with-api-key".to_string()]
    );
    assert_eq!(
        invocation.envs,
        vec![(
            "CODEX_HOME".to_string(),
            "/tmp/modex-test/.modex/api".to_string()
        )]
    );
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
    assert!(script.contains("repeat with attempt from 1 to 50"));
    assert!(script.contains(r#"if application "Codex" is not running then exit repeat"#));
    assert!(script.contains("delay 0.1"));
    assert!(script.contains(r#"error "Codex did not quit" number -128"#));
}

#[cfg(target_os = "macos")]
#[test]
fn macos_quit_script_is_valid_applescript() {
    let script = format!(
        "if false then\n{}\nend if",
        macos_quit_codex_app_script("Finder")
    );

    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .expect("osascript should be available on macOS");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

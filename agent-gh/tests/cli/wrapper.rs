use crate::support::CACHED_TOKEN;
use crate::support::Sandbox;
use crate::support::run;
use crate::support::text;

#[test]
fn prints_usage_without_arguments() {
    let output = run(&mut Sandbox::new().command(&[]), "");
    assert!(output.status.success(), "{}", text(&output.stderr));
    let stdout = text(&output.stdout);
    assert!(stdout.starts_with("Usage: agent-gh"), "{stdout}");
}

#[test]
fn self_help_prints_usage() {
    let output = run(&mut Sandbox::new().command(&["self", "--help"]), "");
    assert!(output.status.success(), "{}", text(&output.stderr));
    let stdout = text(&output.stdout);
    assert!(stdout.starts_with("Usage: agent-gh"), "{stdout}");
}

#[test]
fn self_version_prints_package_version() {
    let output = run(&mut Sandbox::new().command(&["self", "--version"]), "");
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(
        text(&output.stdout),
        format!("agent-gh {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn unknown_self_command_is_a_usage_error() {
    let output = run(&mut Sandbox::new().command(&["self", "bogus"]), "");
    assert_eq!(output.status.code(), Some(2));
    let stderr = text(&output.stderr);
    assert!(
        stderr.contains("unknown wrapper command `self bogus`"),
        "{stderr}"
    );
}

#[test]
fn status_reports_configuration_and_cached_expiry() {
    let sandbox = Sandbox::with_cached_token();
    let output = run(&mut sandbox.command(&["self", "status"]), "");
    assert!(output.status.success(), "{}", text(&output.stderr));
    let stdout = text(&output.stdout);
    let config = sandbox.path("config.toml");
    let expected_lines = [
        format!("config: {}", config.display()),
        format!(
            "cache: {}",
            sandbox.path("cache").join("token.toml").display()
        ),
        "app_id: 1".to_owned(),
        "installation_id: 2".to_owned(),
        format!(
            "private_key_path: {}",
            sandbox.path("missing.pem").display()
        ),
        "run_as_user: []".to_owned(),
    ];
    for line in expected_lines {
        assert!(
            stdout.lines().any(|actual| actual == line),
            "{line}\n{stdout}"
        );
    }
    assert!(stdout.contains("token: valid until "), "{stdout}");
    assert!(!stdout.contains(CACHED_TOKEN), "{stdout}");
}

#[test]
fn status_reports_run_as_user_entries() {
    let sandbox = Sandbox::with_cached_token();
    sandbox.write_config_with("run_as_user = [\"pr  create\", \"pr new\"]\n");
    let output = run(&mut sandbox.command(&["self", "status"]), "");
    assert!(output.status.success(), "{}", text(&output.stderr));
    let stdout = text(&output.stdout);
    assert!(
        stdout
            .lines()
            .any(|line| line == r#"run_as_user: ["pr create", "pr new"]"#),
        "{stdout}"
    );
}

#[test]
fn status_reports_missing_configuration() {
    let sandbox = Sandbox::new();
    let output = run(&mut sandbox.command(&["self", "status"]), "");
    assert_eq!(output.status.code(), Some(1));
    let config = sandbox.path("config.toml");
    let stderr = text(&output.stderr);
    assert!(stderr.starts_with("agent-gh: "), "{stderr}");
    assert!(stderr.contains(&config.display().to_string()), "{stderr}");
}

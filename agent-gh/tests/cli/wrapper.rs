use crate::support::AGENT_MARKER;
use crate::support::CACHED_TOKEN;
use crate::support::CO_AUTHOR;
use crate::support::Sandbox;
use crate::support::run;
use crate::support::same_path;
use crate::support::text;
use crate::support::value;

fn status_lines(sandbox: &Sandbox) -> Vec<String> {
    let output = run(&mut sandbox.command(&["self", "status"]), "");
    assert!(output.status.success(), "{}", text(&output.stderr));
    text(&output.stdout).lines().map(str::to_owned).collect()
}

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
fn setup_without_a_profile_is_a_usage_error() {
    let sandbox = Sandbox::configured();
    for args in [
        &["self", "setup-git-hooks"][..],
        &["self", "setup-git-hooks", "test", "extra"],
    ] {
        let output = run(&mut sandbox.command(args), "");
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert_eq!(
            text(&output.stderr),
            "agent-gh: usage: agent-gh self setup-git-hooks <profile>\n"
        );
    }
}

#[test]
fn status_reports_configured_repository() {
    let sandbox = Sandbox::configured();

    let lines = status_lines(&sandbox);

    let cache = sandbox.path("cache");
    assert_eq!(
        value(&lines, "config"),
        sandbox.path("config.toml").display().to_string()
    );
    assert_eq!(value(&lines, "profiles"), "test");
    assert!(
        same_path(value(&lines, "repository"), &sandbox.path("work")),
        "{lines:#?}"
    );
    assert_eq!(value(&lines, "profile"), "test");
    assert_eq!(value(&lines, "app_id"), "1");
    assert_eq!(value(&lines, "installation_id"), "2");
    assert_eq!(
        value(&lines, "private_key_path"),
        sandbox.path("missing.pem").display().to_string()
    );
    assert_eq!(value(&lines, "run_as_user"), "[]");
    assert_eq!(
        value(&lines, "token cache"),
        cache.join("token-1-2.toml").display().to_string()
    );
    assert!(
        value(&lines, "token").starts_with("valid until "),
        "{lines:#?}"
    );
    assert_eq!(
        value(&lines, "identity cache"),
        cache.join("identity-1.toml").display().to_string()
    );
    assert_eq!(value(&lines, "co-author"), CO_AUTHOR);
    assert_eq!(value(&lines, "commit hook"), "not installed");
    assert!(
        lines.iter().all(|line| !line.contains(CACHED_TOKEN)),
        "{lines:#?}"
    );
}

#[test]
fn status_reports_installed_commit_hook() {
    let sandbox = Sandbox::with_hooks();
    assert_eq!(value(&status_lines(&sandbox), "commit hook"), "installed");
}

#[test]
fn status_reports_missing_identity() {
    let sandbox = Sandbox::configured();
    fs_err::remove_file(sandbox.path("cache/identity-1.toml")).expect("identity cache is removed");
    assert_eq!(value(&status_lines(&sandbox), "co-author"), "not cached");
}

#[test]
fn status_reports_run_as_user_entries() {
    let sandbox = Sandbox::configured();
    sandbox.write_config_with("run_as_user = [\"pr  create\", \"pr new\"]\n");
    let lines = status_lines(&sandbox);
    assert_eq!(value(&lines, "run_as_user"), r#"["pr create", "pr new"]"#);
}

#[test]
fn status_reports_unconfigured_repository() {
    let sandbox = Sandbox::new();
    sandbox.write_config();

    let lines = status_lines(&sandbox);

    assert!(
        same_path(value(&lines, "repository"), &sandbox.path("work")),
        "{lines:#?}"
    );
    assert_eq!(
        value(&lines, "profile"),
        "none; run `agent-gh self setup-git-hooks <profile>` in the repository"
    );
    assert_eq!(value(&lines, "commit hook"), "not installed");
    assert!(
        lines.iter().all(|line| !line.starts_with("app_id: ")),
        "{lines:#?}"
    );
}

#[test]
fn status_reports_undefined_profile() {
    let sandbox = Sandbox::configured();
    sandbox.select_profile("work");

    let lines = status_lines(&sandbox);

    assert_eq!(
        value(&lines, "profile"),
        format!(
            "work, which {} does not define",
            sandbox.path("config.toml").display()
        )
    );
}

#[test]
fn status_reports_working_directory_outside_a_repository() {
    let sandbox = Sandbox::configured();
    let output = run(
        sandbox
            .command(&["self", "status"])
            .current_dir(sandbox.path("outside")),
        "",
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    let stdout = text(&output.stdout);
    let lines: Vec<String> = stdout.lines().map(str::to_owned).collect();

    let repository = value(&lines, "repository");
    assert!(
        repository.starts_with("none (") && repository.contains("not a git repository"),
        "{stdout}"
    );
    assert!(value(&lines, "profile").starts_with("none; "), "{stdout}");
    assert!(
        lines.iter().all(|line| !line.starts_with("commit hook: ")),
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

#[test]
fn co_author_prints_the_value_for_an_agent() {
    let sandbox = Sandbox::configured();

    let output = run(
        sandbox
            .command(&["self", "co-author"])
            .env(AGENT_MARKER, "1"),
        "",
    );

    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(
        text(&output.stdout),
        format!(
            "{CO_AUTHOR}
"
        )
    );
    assert_eq!(text(&output.stderr), "");
}

#[test]
fn co_author_prints_nothing_without_an_agent_marker() {
    let sandbox = Sandbox::configured();
    for marker in [None, Some(("CLAUDECODE", "1")), Some((AGENT_MARKER, ""))] {
        let mut command = sandbox.command(&["self", "co-author"]);
        if let Some((name, value)) = marker {
            command.env(name, value);
        }
        let output = run(&mut command, "");
        assert!(
            output.status.success(),
            "{marker:?}: {}",
            text(&output.stderr)
        );
        assert_eq!(text(&output.stdout), "", "{marker:?}");
        assert_eq!(text(&output.stderr), "", "{marker:?}");
    }
}

#[test]
fn co_author_does_not_read_the_configuration_without_an_agent_marker() {
    let sandbox = Sandbox::new();
    let output = run(&mut sandbox.command(&["self", "co-author"]), "");
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stderr), "");
}

#[test]
fn co_author_with_an_argument_is_a_usage_error() {
    let sandbox = Sandbox::configured();
    let output = run(
        sandbox
            .command(&["self", "co-author", ""])
            .env(AGENT_MARKER, "1"),
        "",
    );
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(text(&output.stdout), "");
}

#[test]
fn co_author_reports_failures_on_one_stderr_line_and_exits_zero() {
    let sandbox = Sandbox::configured();
    fs_err::write(sandbox.path("config.toml"), "app_id = 1\n").expect("configuration is written");

    let output = run(
        sandbox
            .command(&["self", "co-author"])
            .env(AGENT_MARKER, "1"),
        "",
    );

    let stderr = text(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert_eq!(text(&output.stdout), "");
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(stderr.starts_with("agent-gh: parsing "), "{stderr}");
    assert!(stderr.contains("unknown field `app_id`"), "{stderr}");
    assert!(
        stderr.ends_with("; committed without the co-author trailer\n"),
        "{stderr}"
    );
}

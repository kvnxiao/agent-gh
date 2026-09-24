use crate::support::CACHED_TOKEN;
use crate::support::Sandbox;
use crate::support::run;
use crate::support::text;
use serde_json::Value;
use serde_json::json;
use std::collections::HashMap;

const PERSONAL_ENV: [(&str, &str); 5] = [
    ("GH_TOKEN", "personal-token"),
    ("GH_HOST", "enterprise.example.com"),
    ("GITHUB_TOKEN", "personal-token"),
    ("GH_ENTERPRISE_TOKEN", "enterprise-token"),
    ("GITHUB_ENTERPRISE_TOKEN", "enterprise-token"),
];

#[test]
fn passes_arguments_stdin_working_directory_and_exit_status() {
    let sandbox = Sandbox::with_cached_token();
    let args = [
        "issue",
        "comment",
        "12",
        "--body",
        "a \"quoted\" body with spaces, é, and --help",
        "--body-file",
        "-",
    ];

    let output = run(
        sandbox.command(&args).env("FAKE_GH_EXIT", "3"),
        "body from stdin",
    );

    assert_eq!(output.status.code(), Some(3));
    assert_eq!(text(&output.stdout), "fake gh stdout\n");
    assert_eq!(text(&output.stderr), "fake gh stderr\n");
    let record = sandbox.record().expect("fake gh ran");
    assert_eq!(record["args"], json!(args));
    assert_eq!(record["stdin"], "body from stdin");
    let cwd = record["cwd"].as_str().expect("cwd is recorded");
    assert_eq!(
        fs_err::canonicalize(cwd).expect("recorded cwd exists"),
        fs_err::canonicalize(sandbox.path("work")).expect("work directory exists")
    );
}

#[test]
fn sets_installation_token_and_removes_other_tokens() {
    let sandbox = Sandbox::with_cached_token();

    let output = run(sandbox.command(&["api", "user"]).envs(PERSONAL_ENV), "");

    assert!(output.status.success(), "{}", text(&output.stderr));
    let record = sandbox.record().expect("fake gh ran");
    assert_eq!(
        record["env"],
        json!({
            "GH_TOKEN": CACHED_TOKEN,
            "GH_HOST": "github.com",
            "GITHUB_TOKEN": Value::Null,
            "GH_ENTERPRISE_TOKEN": Value::Null,
            "GITHUB_ENTERPRISE_TOKEN": Value::Null,
        })
    );
}

#[test]
fn runs_listed_commands_without_a_token_and_with_the_environment_unchanged() {
    let sandbox = Sandbox::new();
    sandbox.write_config_with("run_as_user = [\"pr create\"]\n");
    sandbox.install_fake_gh();
    let args = ["pr", "create", "--fill"];

    let output = run(sandbox.command(&args).envs(PERSONAL_ENV), "");

    assert!(output.status.success(), "{}", text(&output.stderr));
    let record = sandbox.record().expect("fake gh ran");
    assert_eq!(record["args"], json!(args));
    assert_eq!(record["env"], json!(HashMap::from(PERSONAL_ENV)));
}

#[test]
fn runs_unlisted_commands_with_the_installation_token() {
    let sandbox = Sandbox::with_cached_token();
    sandbox.write_config_with("run_as_user = [\"pr create\"]\n");

    let output = run(
        sandbox.command(&["pr", "comment", "1"]).envs(PERSONAL_ENV),
        "",
    );

    assert!(output.status.success(), "{}", text(&output.stderr));
    let record = sandbox.record().expect("fake gh ran");
    assert_eq!(record["env"]["GH_TOKEN"], CACHED_TOKEN);
}

#[test]
fn stops_before_launching_gh_without_configuration() {
    let sandbox = Sandbox::new();
    sandbox.install_fake_gh();

    let output = run(&mut sandbox.command(&["issue", "list"]), "");

    assert_eq!(output.status.code(), Some(1));
    let stderr = text(&output.stderr);
    assert!(stderr.starts_with("agent-gh: "), "{stderr}");
    assert_eq!(sandbox.record(), None);
}

#[test]
fn reports_missing_gh() {
    let sandbox = Sandbox::new();
    sandbox.write_config();
    sandbox.seed_token();

    let output = run(
        sandbox
            .command(&["issue", "list"])
            .env("PATH", sandbox.path("bin")),
        "",
    );

    assert_eq!(output.status.code(), Some(1));
    let stderr = text(&output.stderr);
    assert_eq!(stderr, "agent-gh: gh was not found on PATH\n");
}

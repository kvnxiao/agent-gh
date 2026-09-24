use crate::support::CACHED_TOKEN;
use crate::support::Sandbox;
use crate::support::run;
use crate::support::same_path;
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
    let sandbox = Sandbox::configured();
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
    assert!(same_path(cwd, &sandbox.path("work")), "{cwd}");
}

#[test]
fn sets_installation_token_and_removes_other_tokens() {
    let sandbox = Sandbox::configured();

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
    sandbox.select_profile("test");
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
    let sandbox = Sandbox::configured();
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
fn uses_the_run_as_user_list_of_the_selected_profile() {
    let sandbox = Sandbox::configured();
    sandbox.write_config_with(
        "\n[profiles.personal]\napp_id = 3\ninstallation_id = 4\nprivate_key_path = \"missing.pem\"\nrun_as_user = [\"pr create\"]\n",
    );

    let output = run(sandbox.command(&["pr", "create"]).envs(PERSONAL_ENV), "");

    assert!(output.status.success(), "{}", text(&output.stderr));
    let record = sandbox.record().expect("fake gh ran");
    assert_eq!(record["env"]["GH_TOKEN"], CACHED_TOKEN);
}

#[test]
fn stops_before_launching_gh_without_configuration() {
    let sandbox = Sandbox::new();
    sandbox.select_profile("test");
    sandbox.install_fake_gh();

    let output = run(&mut sandbox.command(&["issue", "list"]), "");

    assert_eq!(output.status.code(), Some(1));
    let stderr = text(&output.stderr);
    assert!(stderr.starts_with("agent-gh: "), "{stderr}");
    assert_eq!(sandbox.record(), None);
}

#[test]
fn stops_in_a_repository_without_a_selected_profile() {
    let sandbox = Sandbox::new();
    sandbox.write_config();
    sandbox.seed_token();
    sandbox.install_fake_gh();

    let output = run(&mut sandbox.command(&["issue", "list"]), "");

    assert_eq!(output.status.code(), Some(1));
    let stderr = text(&output.stderr);
    let Some((root, _)) = stderr
        .strip_prefix("agent-gh: no profile is selected for the repository at ")
        .and_then(|rest| rest.split_once(", so agent-gh did not run gh.\n"))
    else {
        panic!("unexpected message: {stderr}");
    };
    assert!(same_path(root, &sandbox.path("work")), "{stderr}");
    assert!(
        stderr.ends_with(&format!(
            ", so agent-gh did not run gh.\nAsk the user to run `agent-gh self setup-git-hooks <profile>` in that repository. Profiles in {}: test\n",
            sandbox.path("config.toml").display()
        )),
        "{stderr}"
    );
    assert_eq!(sandbox.record(), None);
}

#[test]
fn stops_outside_any_repository() {
    let sandbox = Sandbox::configured();

    let output = run(
        sandbox
            .command(&["issue", "list"])
            .current_dir(sandbox.path("outside")),
        "",
    );

    assert_eq!(output.status.code(), Some(1));
    let stderr = text(&output.stderr);
    assert!(
        stderr.starts_with(
            "agent-gh: the profile comes from the Git repository in the working directory, and Git found no usable repository there, so agent-gh did not run gh. Git reported:\n"
        ),
        "{stderr}"
    );
    assert!(
        stderr.contains("--local can only be used inside a git repository"),
        "{stderr}"
    );
    assert!(
        stderr.contains("Run the command in a repository where the user ran `agent-gh self setup-git-hooks <profile>`."),
        "{stderr}"
    );
    assert_eq!(sandbox.record(), None);
}

#[test]
fn stops_when_the_selected_profile_is_not_defined() {
    let sandbox = Sandbox::configured();
    sandbox.select_profile("work");

    let output = run(&mut sandbox.command(&["issue", "list"]), "");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        text(&output.stderr),
        format!(
            "agent-gh: the repository selects an undefined profile: {} does not define profile `work`; defined profiles: test\n",
            sandbox.path("config.toml").display()
        )
    );
    assert_eq!(sandbox.record(), None);
}

#[test]
fn reports_missing_gh() {
    let sandbox = Sandbox::new();
    sandbox.write_config();
    sandbox.select_profile("test");
    sandbox.seed_token();

    let output = run(
        sandbox
            .command(&["issue", "list"])
            .env("PATH", Sandbox::path_without_gh()),
        "",
    );

    assert_eq!(output.status.code(), Some(1));
    let stderr = text(&output.stderr);
    assert_eq!(stderr, "agent-gh: gh was not found on PATH\n");
}

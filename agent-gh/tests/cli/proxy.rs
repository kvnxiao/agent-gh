use crate::support::CACHED_TOKEN;
use crate::support::Sandbox;
use crate::support::run;
use crate::support::same_path;
use crate::support::text;
use serde_json::Value;
use serde_json::json;
use std::collections::HashMap;

const PERSONAL_TOKEN: &str = "personal-token";
const PERSONAL_ENV: [(&str, &str); 5] = [
    ("GH_TOKEN", PERSONAL_TOKEN),
    ("GH_HOST", "enterprise.example.com"),
    ("GITHUB_TOKEN", PERSONAL_TOKEN),
    ("GH_ENTERPRISE_TOKEN", "enterprise-token"),
    ("GITHUB_ENTERPRISE_TOKEN", "enterprise-token"),
];

#[test]
fn passes_arguments_stdin_working_directory_and_exit_status_on_both_paths() {
    let args = [
        "issue",
        "comment",
        "12",
        "--body",
        "a \"quoted\" body with spaces, é, and --help",
        "--body-file",
        "-",
    ];
    for (run_as_bot, expected_token) in [
        ("run_as_bot = [\"issue comment\"]\n", CACHED_TOKEN),
        ("", PERSONAL_TOKEN),
    ] {
        let sandbox = Sandbox::configured();
        sandbox.write_config_with(run_as_bot);
        fs_err::create_dir(sandbox.path("work/sub")).expect("subdirectory is created");

        let output = run(
            sandbox
                .command(&args)
                .current_dir(sandbox.path("work/sub"))
                .env("GH_TOKEN", PERSONAL_TOKEN)
                .env("FAKE_GH_EXIT", "3"),
            "body from stdin",
        );

        assert_eq!(output.status.code(), Some(3), "{run_as_bot:?}");
        assert_eq!(text(&output.stdout), "fake gh stdout\n", "{run_as_bot:?}");
        assert_eq!(text(&output.stderr), "fake gh stderr\n", "{run_as_bot:?}");
        let records = sandbox.records();
        let [record] = records.as_slice() else {
            panic!("{run_as_bot:?}: expected one gh invocation, got {records:#?}");
        };
        assert_eq!(record["args"], json!(args), "{run_as_bot:?}");
        assert_eq!(record["stdin"], "body from stdin", "{run_as_bot:?}");
        assert_eq!(record["env"]["GH_TOKEN"], expected_token, "{run_as_bot:?}");
        let cwd = record["cwd"].as_str().expect("cwd is recorded");
        assert!(same_path(cwd, &sandbox.path("work/sub")), "{cwd}");
    }
}

#[test]
fn bot_path_sets_installation_token_and_removes_other_tokens() {
    let sandbox = Sandbox::configured();
    sandbox.write_config_with("run_as_bot = [\"api user\"]\n");

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
fn user_path_keeps_the_environment_and_reads_no_app_credentials() {
    let sandbox = Sandbox::new();
    sandbox.write_config_with("run_as_bot = [\"pr comment\"]\n");
    sandbox.select_profile("test");
    sandbox.install_fake_gh();
    // A directory at the token cache path makes every read of the cache fail,
    // and the profile's private key does not exist.
    fs_err::create_dir(sandbox.path("cache/token-1-2.toml")).expect("cache trap is created");
    let args = ["pr", "create", "--fill"];

    let user = run(sandbox.command(&args).envs(PERSONAL_ENV), "");

    assert!(user.status.success(), "{}", text(&user.stderr));
    let record = sandbox.record().expect("fake gh ran");
    assert_eq!(record["args"], json!(args));
    assert_eq!(record["env"], json!(HashMap::from(PERSONAL_ENV)));

    let bot = run(
        sandbox.command(&["pr", "comment", "1"]).envs(PERSONAL_ENV),
        "",
    );

    assert_eq!(bot.status.code(), Some(1), "{}", text(&bot.stderr));
    assert_eq!(sandbox.records().len(), 1);
}

#[test]
fn failed_token_acquisition_does_not_run_gh() {
    let sandbox = Sandbox::new();
    sandbox.write_config_with("run_as_bot = [\"issue comment\"]\n");
    sandbox.select_profile("test");
    sandbox.install_fake_gh();

    let output = run(
        sandbox
            .command(&["issue", "comment", "1", "--body", "hi"])
            .envs(PERSONAL_ENV),
        "",
    );

    assert_eq!(output.status.code(), Some(1));
    let stderr = text(&output.stderr);
    assert!(
        stderr.starts_with("agent-gh: ") && stderr.contains("missing.pem"),
        "{stderr}"
    );
    assert_eq!(sandbox.records(), Vec::<Value>::new());
}

#[test]
fn uses_the_run_as_bot_rules_of_the_selected_profile() {
    let sandbox = Sandbox::configured();
    sandbox.write_config_with(
        "\n[profiles.personal]\napp_id = 3\ninstallation_id = 4\nprivate_key_path = \"missing.pem\"\nrun_as_bot = [\"pr create\"]\n",
    );
    sandbox.seed_token_for(3, 4);

    for (profile, expected_token) in [("test", PERSONAL_TOKEN), ("personal", CACHED_TOKEN)] {
        sandbox.select_profile(profile);

        let output = run(sandbox.command(&["pr", "create"]).envs(PERSONAL_ENV), "");

        assert!(
            output.status.success(),
            "{profile}: {}",
            text(&output.stderr)
        );
        let record = sandbox.record().expect("fake gh ran");
        assert_eq!(record["env"]["GH_TOKEN"], expected_token, "{profile}");
    }
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
            ", so agent-gh did not run gh.\nAsk the user to run `agent-gh self setup <profile>` in that repository. Profiles in {}: test\n",
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
        stderr.contains(
            "Run the command in a repository where the user ran `agent-gh self setup <profile>`."
        ),
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

use crate::support::CACHED_TOKEN;
use crate::support::Sandbox;
use crate::support::run;
use crate::support::text;
use serde_json::Value;
use serde_json::json;
use std::process::Command;

const USER_TOKEN: &str = "user-token";
const COMMENTS: &str = r#"run_as_bot = [
  "issue comment",
  "pr comment",
  "pr review --comment",
  "pr review -c",
  "api repos/*/comments",
  "api repos/*/comments/*",
]
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Identity {
    Bot,
    User,
}

use Identity::Bot;
use Identity::User;

fn words(command: &str) -> Vec<&str> {
    command.split(' ').collect()
}

fn run_once(sandbox: &Sandbox, command: &mut Command, args: &[&str]) -> Value {
    let before = sandbox.records().len();

    let output = run(command.env("GH_TOKEN", USER_TOKEN), "");

    assert!(
        output.status.success(),
        "{args:?}: {}",
        text(&output.stderr)
    );
    let mut records = sandbox.records();
    assert_eq!(records.len(), before + 1, "{args:?}: one gh invocation");
    let record = records.pop().expect("fake gh ran");
    assert_eq!(record["args"], json!(args), "{args:?}");
    record
}

fn identity(record: &Value) -> Identity {
    match record["env"]["GH_TOKEN"].as_str() {
        Some(CACHED_TOKEN) => Bot,
        Some(USER_TOKEN) => User,
        other => panic!("unexpected GH_TOKEN {other:?} in {record}"),
    }
}

fn assert_identities(run_as_bot: &str, cases: &[(&str, Identity)]) {
    let sandbox = Sandbox::configured();
    sandbox.write_config_with(run_as_bot);
    for &(command, expected) in cases {
        let args = words(command);
        let record = run_once(&sandbox, &mut sandbox.command(&args), &args);
        assert_eq!(identity(&record), expected, "{command}");
    }
}

#[test]
fn comments_configuration_selects_the_accepted_identities() {
    assert_identities(
        COMMENTS,
        &[
            ("issue comment 1 --body hi", Bot),
            ("pr comment 1 --body hi", Bot),
            ("pr review 1 --comment --body hi", Bot),
            ("pr review 1 -c -b hi", Bot),
            ("issue comment 1 --edit-last --body x", Bot),
            ("issue comment 1 --delete-last", Bot),
            ("api repos/o/r/issues/1/comments -f body=x", Bot),
            ("api repos/o/r/pulls/1/comments -f body=x", Bot),
            ("api repos/o/r/pulls/1/comments/2/replies -f body=x", Bot),
            ("api repos/o/r/issues/comments/3 -X PATCH -f body=x", Bot),
            ("api -X PATCH repos/o/r/issues/comments/3 -f body=x", User),
            ("api repos/o/r/issues/1/comments", Bot),
            ("pr review 1 --approve", User),
            ("pr review 1 --request-changes -b x", User),
            ("pr review 1 --approve --body -c", Bot),
            ("pr -R o/r review 1 --comment -b x", User),
            ("api -X POST repos/o/r/issues/1/comments", User),
            ("pr review 1 -bc hi", User),
            ("api /repos/o/r/issues/1/comments -f body=x", User),
            ("pr new", User),
            ("issue new", User),
            ("pr create", User),
            ("repo view", User),
            ("issue comment 1 --web", Bot),
            ("api repos/o/r/issues/comments/3 -X DELETE", Bot),
            ("pr comment 1 --edit-last --body x", Bot),
            ("pr comment 1 --delete-last", Bot),
            ("pr view 1 --web", User),
            ("co 1", User),
            ("my-ext comment", User),
        ],
    );
}

#[test]
fn rules_do_not_inspect_the_host() {
    let sandbox = Sandbox::configured();
    sandbox.write_config_with(COMMENTS);
    let args = words("issue comment 1 -R ghe.example.com/o/r --body x");

    let record = run_once(
        &sandbox,
        sandbox.command(&args).env("GH_HOST", "ghe.example.com"),
        &args,
    );

    assert_eq!(identity(&record), Bot);
    assert_eq!(record["env"]["GH_HOST"], "github.com");
}

#[test]
fn rules_apply_in_every_repository_that_selects_the_profile() {
    let sandbox = Sandbox::configured();
    sandbox.write_config_with("run_as_bot = [\"issue comment\"]\n");
    sandbox.git_ok(&["init", "--quiet", "../second"]);
    sandbox.git_ok(&[
        "-C",
        "../second",
        "config",
        "--local",
        "agent-gh.profile",
        "test",
    ]);
    sandbox.git_ok(&["commit", "--quiet", "--allow-empty", "-m", "initial"]);
    sandbox.git_ok(&["worktree", "add", "--quiet", "../linked"]);
    let args = words("issue comment 1");

    for dir in ["work", "second", "linked"] {
        let record = run_once(
            &sandbox,
            sandbox.command(&args).current_dir(sandbox.path(dir)),
            &args,
        );

        assert_eq!(identity(&record), Bot, "{dir}");
    }
}

#[test]
fn the_working_repository_selects_the_profile_whatever_the_target() {
    let sandbox = Sandbox::configured();
    sandbox.write_config_with(
        "run_as_bot = [\"issue comment\"]\n\n[profiles.other]\napp_id = 3\ninstallation_id = 4\nprivate_key_path = \"missing.pem\"\nrun_as_bot = [\"issue view\"]\n",
    );
    sandbox.seed_token_for(3, 4);
    sandbox.git_ok(&["commit", "--quiet", "--allow-empty", "-m", "initial"]);
    sandbox.git_ok(&["worktree", "add", "--quiet", "../linked"]);
    let cases = [
        ("linked", None, "issue comment 1", Bot),
        ("linked", None, "issue view 1", User),
        ("work", Some("other/repo"), "issue comment 1", Bot),
        ("work", Some("other/repo"), "issue view 1", User),
        ("work", None, "issue comment 1 -R other/repo", Bot),
        ("work", None, "issue view 1 -R other/repo", User),
    ];

    for (dir, gh_repo, command, expected) in cases {
        let args = words(command);
        let mut gh = sandbox.command(&args);
        gh.current_dir(sandbox.path(dir));
        if let Some(repo) = gh_repo {
            gh.env("GH_REPO", repo);
        }

        let record = run_once(&sandbox, &mut gh, &args);

        assert_eq!(identity(&record), expected, "{dir} {gh_repo:?} {command}");
    }
}

#[test]
fn a_non_comment_entry_selects_the_bot() {
    assert_identities(
        "run_as_bot = [\"label create\"]\n",
        &[
            ("label create bug", Bot),
            ("label list", User),
            ("issue comment 1 --body hi", User),
        ],
    );
}

#[test]
fn omitted_and_empty_run_as_bot_select_the_user() {
    for run_as_bot in ["", "run_as_bot = []\n"] {
        assert_identities(
            run_as_bot,
            &[
                ("issue comment 1 --body hi", User),
                ("pr review 1 -c -b hi", User),
            ],
        );
    }
}

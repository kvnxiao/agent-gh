use crate::support::AGENT_MARKER;
use crate::support::CO_AUTHOR;
use crate::support::Sandbox;
use crate::support::run;
use crate::support::same_path;
use crate::support::text;
use std::process::Output;

const HOOK_KEY_PATTERN: &str = r"^(agent-gh|hook\.agent-gh\.commit-msg)\.";
const HOOK_KEYS: &[&str] = &[
    "hook.agent-gh.commit-msg.event commit-msg",
    r#"hook.agent-gh.commit-msg.command git interpret-trailers --in-place --trim-empty --if-exists addIfDifferent --trailer "Co-authored-by: $(agent-gh self co-author)""#,
];
const OTHER_PROFILE: &str =
    "\n[profiles.other]\napp_id = 3\ninstallation_id = 4\nprivate_key_path = \"missing.pem\"\n";
const OTHER_CO_AUTHOR: &str = "other-app[bot] <43+other-app[bot]@users.noreply.github.com>";

fn hook_keys(sandbox: &Sandbox) -> Vec<String> {
    let output = run(
        &mut sandbox.git(&["config", "--local", "--get-regexp", HOOK_KEY_PATTERN]),
        "",
    );
    text(&output.stdout).lines().map(str::to_owned).collect()
}

fn expected_keys(profile: &str) -> Vec<String> {
    std::iter::once(format!("agent-gh.profile {profile}"))
        .chain(HOOK_KEYS.iter().map(|&key| key.to_owned()))
        .collect()
}

fn co_authors(message: &str) -> Vec<&str> {
    message
        .lines()
        .filter_map(|line| line.strip_prefix("Co-authored-by: "))
        .collect()
}

fn unconfigured() -> Sandbox {
    let sandbox = Sandbox::new();
    sandbox.write_config();
    sandbox.seed_token();
    sandbox.seed_identity();
    sandbox
}

fn assert_committed(output: &Output) {
    assert!(
        output.status.success(),
        "commit failed: {}",
        text(&output.stderr)
    );
}

#[test]
fn setup_writes_hook_keys_and_prints_co_author() {
    let sandbox = unconfigured();

    let stdout = sandbox.setup_git_hooks("test");

    assert_eq!(hook_keys(&sandbox), expected_keys("test"));
    let lines: Vec<&str> = stdout.lines().collect();
    let [repository, profile, co_author] = lines.as_slice() else {
        panic!("unexpected output: {stdout}");
    };
    let root = repository
        .strip_prefix("repository: ")
        .expect("setup prints the repository");
    assert!(same_path(root, &sandbox.path("work")), "{stdout}");
    assert_eq!(*profile, "profile: test");
    assert_eq!(*co_author, format!("co-author: {CO_AUTHOR}"));
}

#[test]
fn setup_is_idempotent() {
    let sandbox = unconfigured();
    sandbox.setup_git_hooks("test");
    sandbox.setup_git_hooks("test");
    assert_eq!(hook_keys(&sandbox), expected_keys("test"));
}

#[test]
fn setup_switches_the_repository_to_another_profile() {
    let sandbox = unconfigured();
    sandbox.write_config_with(OTHER_PROFILE);
    sandbox.seed_token_for(3, 4);
    sandbox.seed_identity_for(3, "other-app", 43);
    sandbox.setup_git_hooks("test");

    sandbox.setup_git_hooks("other");

    assert_eq!(hook_keys(&sandbox), expected_keys("other"));
    assert_committed(&sandbox.agent_commit(&["-m", "subject"]));
    assert_eq!(co_authors(&sandbox.last_message()), [OTHER_CO_AUTHOR]);
}

#[test]
fn setup_with_an_undefined_profile_writes_nothing() {
    let sandbox = unconfigured();

    let output = run(
        &mut sandbox.command(&["self", "setup-git-hooks", "work"]),
        "",
    );

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        text(&output.stderr),
        format!(
            "agent-gh: {} does not define profile `work`; defined profiles: test\n",
            sandbox.path("config.toml").display()
        )
    );
    assert_eq!(hook_keys(&sandbox), Vec::<String>::new());
}

#[test]
fn setup_that_cannot_obtain_a_token_writes_nothing() {
    let sandbox = unconfigured();
    fs_err::remove_file(sandbox.path("cache/token-1-2.toml")).expect("token cache is removed");

    let output = run(
        &mut sandbox.command(&["self", "setup-git-hooks", "test"]),
        "",
    );

    assert_eq!(output.status.code(), Some(1));
    let stderr = text(&output.stderr);
    assert!(stderr.contains("missing.pem"), "{stderr}");
    assert_eq!(hook_keys(&sandbox), Vec::<String>::new());
}

#[test]
fn setup_outside_a_repository_fails() {
    let sandbox = unconfigured();

    let output = run(
        sandbox
            .command(&["self", "setup-git-hooks", "test"])
            .current_dir(sandbox.path("outside")),
        "",
    );

    assert_eq!(output.status.code(), Some(1));
    let stderr = text(&output.stderr);
    assert!(
        stderr.starts_with("agent-gh: setup-git-hooks must run in a Git working tree: "),
        "{stderr}"
    );
}

#[test]
fn agent_commit_gets_one_bot_trailer() {
    let sandbox = Sandbox::with_hooks();

    let output = sandbox.agent_commit(&["-m", "subject"]);

    assert_committed(&output);
    assert_eq!(text(&output.stderr), "");
    assert_eq!(
        sandbox.last_message(),
        format!("subject\n\nCo-authored-by: {CO_AUTHOR}\n\n")
    );
}

#[test]
fn amending_an_agent_commit_keeps_one_bot_trailer() {
    let sandbox = Sandbox::with_hooks();
    assert_committed(&sandbox.agent_commit(&["-m", "subject"]));

    assert_committed(&sandbox.agent_commit(&["--amend", "--no-edit"]));

    assert_eq!(co_authors(&sandbox.last_message()), [CO_AUTHOR]);
}

#[test]
fn agent_commit_keeps_existing_co_author_trailers() {
    let sandbox = Sandbox::with_hooks();

    let output = sandbox.agent_commit(&[
        "-m",
        "subject",
        "-m",
        "Co-Authored-By: Claude <noreply@anthropic.com>",
    ]);

    assert_committed(&output);
    assert_eq!(
        sandbox.last_message(),
        format!(
            "subject\n\nCo-Authored-By: Claude <noreply@anthropic.com>\nCo-authored-by: {CO_AUTHOR}\n\n"
        )
    );
}

#[test]
fn commit_keeps_co_authors_from_the_trailer_option() {
    let sandbox = Sandbox::with_hooks();
    let alice = "Alice <alice@example.com>";
    let trailer = format!("Co-authored-by: {alice}");

    assert_committed(&run(
        &mut sandbox.commit(&["--trailer", &trailer, "-m", "human"]),
        "",
    ));
    assert_eq!(co_authors(&sandbox.last_message()), [alice]);

    assert_committed(&sandbox.agent_commit(&["--trailer", &trailer, "-m", "agent"]));
    assert_eq!(co_authors(&sandbox.last_message()), [alice, CO_AUTHOR]);
}

#[test]
fn commit_keeps_trailer_keys_that_prefix_the_hook_names() {
    let sandbox = Sandbox::with_hooks();

    let output = run(
        &mut sandbox.commit(&["-m", "subject", "-m", "Agent: Claude Code\nCo: x"]),
        "",
    );

    assert_committed(&output);
    assert_eq!(
        sandbox.last_message(),
        "subject\n\nAgent: Claude Code\nCo: x\n\n"
    );
}

#[test]
fn commit_without_an_agent_marker_gets_no_trailer() {
    let sandbox = Sandbox::with_hooks();

    let output = run(&mut sandbox.commit(&["-m", "subject"]), "");

    assert_committed(&output);
    assert_eq!(text(&output.stderr), "");
    assert_eq!(sandbox.last_message(), "subject\n\n");
}

#[test]
fn commit_with_only_claudecode_set_gets_no_trailer() {
    let sandbox = Sandbox::with_hooks();

    let output = run(
        sandbox.commit(&["-m", "subject"]).env("CLAUDECODE", "1"),
        "",
    );

    assert_committed(&output);
    assert_eq!(sandbox.last_message(), "subject\n\n");
}

#[test]
fn identity_fetch_failure_commits_without_the_trailer() {
    let sandbox = Sandbox::with_hooks();
    fs_err::remove_file(sandbox.path("cache/identity-1.toml")).expect("identity cache is removed");

    let output = sandbox.agent_commit(&["-m", "subject"]);

    assert_committed(&output);
    let stderr = text(&output.stderr);
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(
        stderr.starts_with("agent-gh: looking up the App slug: "),
        "{stderr}"
    );
    assert!(
        stderr.ends_with("; committed without the co-author trailer\n"),
        "{stderr}"
    );
    assert_eq!(sandbox.last_message(), "subject\n\n");
}

#[test]
fn missing_configuration_commits_without_the_trailer() {
    let sandbox = Sandbox::with_hooks();
    fs_err::remove_file(sandbox.path("config.toml")).expect("configuration is removed");

    let output = sandbox.agent_commit(&["-m", "subject"]);

    assert_committed(&output);
    let stderr = text(&output.stderr);
    assert!(
        stderr.contains("config.toml")
            && stderr.ends_with("; committed without the co-author trailer\n"),
        "{stderr}"
    );
    assert_eq!(sandbox.last_message(), "subject\n\n");
}

#[test]
fn commit_without_agent_gh_on_path_gets_no_trailer() {
    let sandbox = Sandbox::with_hooks();

    let output = run(
        sandbox
            .commit(&["-m", "subject"])
            .env(AGENT_MARKER, "1")
            .env("PATH", sandbox.path_without_agent_gh()),
        "",
    );

    assert_committed(&output);
    assert_eq!(sandbox.last_message(), "subject\n\n");
}

#[test]
fn hook_removes_empty_trailers_from_every_commit() {
    let sandbox = Sandbox::with_hooks();

    let output = run(&mut sandbox.commit(&["-m", "subject", "-m", "Fixes:"]), "");

    assert_committed(&output);
    assert_eq!(sandbox.last_message(), "subject\n\n");
}

#[test]
fn agent_commit_in_a_linked_worktree_gets_the_trailer() {
    let sandbox = Sandbox::with_hooks();
    assert_committed(&sandbox.agent_commit(&["-m", "initial"]));
    sandbox.git_ok(&["worktree", "add", "--quiet", "../linked"]);
    let linked = sandbox.path("linked");

    let output = run(
        sandbox
            .commit(&["-m", "subject"])
            .current_dir(&linked)
            .env(AGENT_MARKER, "1"),
        "",
    );

    assert_committed(&output);
    let message = run(
        sandbox
            .git(&["log", "-1", "--format=%B"])
            .current_dir(&linked),
        "",
    );
    assert_eq!(
        text(&message.stdout),
        format!("subject\n\nCo-authored-by: {CO_AUTHOR}\n\n")
    );
}

#[test]
fn agent_commit_with_a_separate_work_tree_gets_the_trailer() {
    let sandbox = Sandbox::with_hooks();
    let git_dir = sandbox.path("work/.git");
    let git_dir = git_dir.to_str().expect("sandbox path is UTF-8");

    let output = run(
        sandbox
            .git(&[
                "--git-dir",
                git_dir,
                "--work-tree",
                ".",
                "commit",
                "--quiet",
                "--allow-empty",
                "-m",
                "subject",
            ])
            .current_dir(sandbox.path("outside"))
            .env(AGENT_MARKER, "1"),
        "",
    );

    assert_committed(&output);
    assert_eq!(text(&output.stderr), "");
    assert_eq!(
        sandbox.last_message(),
        format!("subject\n\nCo-authored-by: {CO_AUTHOR}\n\n")
    );
}

#[test]
fn remove_git_hooks_removes_the_keys_and_the_selection() {
    let sandbox = Sandbox::with_hooks();

    let output = run(&mut sandbox.command(&["self", "remove-git-hooks"]), "");

    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(hook_keys(&sandbox), Vec::<String>::new());
    assert_committed(&sandbox.agent_commit(&["-m", "subject"]));
    assert_eq!(sandbox.last_message(), "subject\n\n");
    let proxied = run(&mut sandbox.command(&["issue", "list"]), "");
    assert_eq!(proxied.status.code(), Some(1));
    let stderr = text(&proxied.stderr);
    assert!(
        stderr.starts_with("agent-gh: no profile is selected for the repository at "),
        "{stderr}"
    );
    assert_eq!(sandbox.record(), None);
}

#[test]
fn remove_git_hooks_succeeds_when_nothing_is_installed() {
    let sandbox = Sandbox::new();

    let output = run(&mut sandbox.command(&["self", "remove-git-hooks"]), "");

    assert!(output.status.success(), "{}", text(&output.stderr));
}

#[test]
fn remove_git_hooks_fails_outside_a_repository() {
    let sandbox = Sandbox::new();

    let output = run(
        sandbox
            .command(&["self", "remove-git-hooks"])
            .current_dir(sandbox.path("outside")),
        "",
    );

    assert_eq!(output.status.code(), Some(1));
    let stderr = text(&output.stderr);
    assert!(
        stderr.contains("--local can only be used inside a git repository"),
        "{stderr}"
    );
}

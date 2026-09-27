use crate::support::AGENT_MARKER;
use crate::support::CACHED_TOKEN;
use crate::support::CO_AUTHOR;
use crate::support::Sandbox;
use crate::support::run;
use crate::support::same_path;
use crate::support::text;
use std::iter;
use std::process::Output;

const CONFIG_KEY_PATTERN: &str = r"^(agent-gh|hook)\.";
const HOOK_KEYS: &[&str] = &[
    "hook.agent-gh.commit-msg.event commit-msg",
    r#"hook.agent-gh.commit-msg.command git interpret-trailers --in-place --trim-empty --if-exists addIfDifferent --trailer "Co-authored-by: $(agent-gh self co-author)""#,
];
const UNRELATED_HOOK: [(&str, &str); 2] = [
    ("hook.lint.event", "pre-commit"),
    ("hook.lint.command", "true"),
];
const OTHER_PROFILE: &str =
    "\n[profiles.other]\napp_id = 3\ninstallation_id = 4\nprivate_key_path = \"missing.pem\"\n";
const OTHER_CO_AUTHOR: &str = "other-app[bot] <43+other-app[bot]@users.noreply.github.com>";
const VERSION_CHECK: &str = "built-in: git version";
const SETUP_MODES: [&[&str]; 2] = [&[], &["--git-hooks"]];
const ISSUE_LIST_AS_BOT: &str = "run_as_bot = [\"issue list\"]\n";

fn traced(sandbox: &Sandbox, args: &[&str]) -> (Output, String) {
    let trace = sandbox.path("trace.txt");
    let output = run(sandbox.command(args).env("GIT_TRACE", &trace), "");
    let trace = fs_err::read_to_string(trace).expect("git writes the trace");
    (output, trace)
}

fn cache_entries(sandbox: &Sandbox) -> Vec<String> {
    fs_err::read_dir(sandbox.path("cache"))
        .expect("cache directory is readable")
        .map(|entry| {
            let entry = entry.expect("cache entry is readable");
            entry.file_name().to_string_lossy().into_owned()
        })
        .collect()
}

fn assert_setup_output(sandbox: &Sandbox, worktree: &str, stdout: &str, expected: &[&str]) {
    let lines: Vec<&str> = stdout.lines().collect();
    let Some((repository, rest)) = lines.split_first() else {
        panic!("setup printed nothing");
    };
    let root = repository
        .strip_prefix("repository: ")
        .unwrap_or_else(|| panic!("setup prints the repository first: {stdout}"));
    assert!(same_path(root, &sandbox.path(worktree)), "{stdout}");
    assert_eq!(rest, expected, "{stdout}");
}

fn assert_issue_list_uses_the_profile(sandbox: &Sandbox) {
    let proxied = run(&mut sandbox.command(&["issue", "list"]), "");
    assert!(proxied.status.success(), "{}", text(&proxied.stderr));
    let record = sandbox.record().expect("fake gh ran");
    assert_eq!(record["env"]["GH_TOKEN"], CACHED_TOKEN);

    // The profile's private key does not exist, so refresh fails at token
    // issuance after it resolves the profile.
    let refresh = run(&mut sandbox.command(&["self", "refresh"]), "");
    assert_eq!(refresh.status.code(), Some(1));
    let stderr = text(&refresh.stderr);
    assert!(
        stderr.contains("missing.pem") && !stderr.contains("no profile is selected"),
        "{stderr}"
    );
}

fn configured_keys(sandbox: &Sandbox) -> Vec<String> {
    let output = run(
        &mut sandbox.git(&["config", "--local", "--get-regexp", CONFIG_KEY_PATTERN]),
        "",
    );
    sorted(text(&output.stdout).lines().map(str::to_owned))
}

fn sorted(keys: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut keys: Vec<String> = keys.into_iter().collect();
    keys.sort();
    keys
}

fn selection(profile: &str) -> String {
    format!("agent-gh.profile {profile}")
}

fn hook_keys() -> impl Iterator<Item = String> {
    HOOK_KEYS.iter().map(|&key| key.to_owned())
}

fn unrelated_keys() -> impl Iterator<Item = String> {
    UNRELATED_HOOK
        .iter()
        .map(|(key, value)| format!("{key} {value}"))
}

fn expected_keys(profile: &str) -> Vec<String> {
    sorted(iter::once(selection(profile)).chain(hook_keys()))
}

fn add_unrelated_hook(sandbox: &Sandbox) {
    for (key, value) in UNRELATED_HOOK {
        sandbox.git_ok(&["config", "--local", key, value]);
    }
}

fn with_mode<'a>(profile: &'a str, mode: &[&'a str]) -> Vec<&'a str> {
    [&[profile][..], mode].concat()
}

fn setup_command<'a>(profile: &'a str, mode: &[&'a str]) -> Vec<&'a str> {
    [&["self", "setup"][..], &with_mode(profile, mode)].concat()
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

fn with_other_profile() -> Sandbox {
    let sandbox = unconfigured();
    sandbox.write_config_with(OTHER_PROFILE);
    sandbox.seed_token_for(3, 4);
    sandbox.seed_identity_for(3, "other-app", 43);
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
fn setup_selects_the_profile_without_hooks_or_github_requests() {
    let sandbox = Sandbox::new();
    sandbox.write_config();

    let (output, trace) = traced(&sandbox, &["self", "setup", "test"]);

    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_setup_output(
        &sandbox,
        "work",
        &text(&output.stdout),
        &["profile: test", "commit hook: not installed"],
    );
    assert_eq!(configured_keys(&sandbox), [selection("test")]);
    assert_eq!(cache_entries(&sandbox), Vec::<String>::new());
    assert!(trace.contains("built-in: git config"), "{trace}");
    assert!(!trace.contains(VERSION_CHECK), "{trace}");
}

#[test]
fn hook_installation_checks_the_git_version() {
    for args in [
        &["self", "setup", "test", "--git-hooks"][..],
        &["self", "install-git-hooks"],
    ] {
        let sandbox = unconfigured();
        sandbox.select_profile("test");

        let (output, trace) = traced(&sandbox, args);

        assert!(
            output.status.success(),
            "{args:?}: {}",
            text(&output.stderr)
        );
        assert!(trace.contains(VERSION_CHECK), "{args:?}: {trace}");
    }
}

#[test]
fn setup_with_git_hooks_writes_hook_keys_and_prints_co_author() {
    for args in [["test", "--git-hooks"], ["--git-hooks", "test"]] {
        let sandbox = unconfigured();

        let stdout = sandbox.setup(&args);

        assert_eq!(configured_keys(&sandbox), expected_keys("test"), "{args:?}");
        assert_setup_output(
            &sandbox,
            "work",
            &stdout,
            &[
                "profile: test",
                "commit hook: installed",
                &format!("co-author: {CO_AUTHOR}"),
            ],
        );
    }
}

#[test]
fn setup_is_idempotent() {
    for (mode, expected) in SETUP_MODES
        .into_iter()
        .zip([vec![selection("test")], expected_keys("test")])
    {
        let sandbox = unconfigured();
        sandbox.setup(&with_mode("test", mode));
        sandbox.setup(&with_mode("test", mode));
        assert_eq!(configured_keys(&sandbox), expected, "{mode:?}");
    }
}

#[test]
fn setup_with_git_hooks_switches_the_repository_to_another_profile() {
    let sandbox = with_other_profile();
    sandbox.setup(&["test", "--git-hooks"]);

    sandbox.setup(&["other", "--git-hooks"]);

    assert_eq!(configured_keys(&sandbox), expected_keys("other"));
    assert_committed(&sandbox.agent_commit(&["-m", "subject"]));
    assert_eq!(co_authors(&sandbox.last_message()), [OTHER_CO_AUTHOR]);
}

#[test]
fn setup_without_git_hooks_switches_the_profile_and_keeps_the_hook() {
    let sandbox = with_other_profile();
    sandbox.setup(&["test", "--git-hooks"]);

    let stdout = sandbox.setup(&["other"]);

    assert_setup_output(
        &sandbox,
        "work",
        &stdout,
        &["profile: other", "commit hook: installed"],
    );
    assert_eq!(configured_keys(&sandbox), expected_keys("other"));
    assert_committed(&sandbox.agent_commit(&["-m", "subject"]));
    assert_eq!(co_authors(&sandbox.last_message()), [OTHER_CO_AUTHOR]);
}

#[test]
fn setup_keeps_unrelated_hooks() {
    let sandbox = unconfigured();
    add_unrelated_hook(&sandbox);

    sandbox.setup(&["test"]);
    assert_eq!(
        configured_keys(&sandbox),
        sorted(iter::once(selection("test")).chain(unrelated_keys()))
    );

    sandbox.setup(&["test", "--git-hooks"]);
    assert_eq!(
        configured_keys(&sandbox),
        sorted(
            iter::once(selection("test"))
                .chain(hook_keys())
                .chain(unrelated_keys())
        )
    );
}

#[test]
fn setup_in_a_linked_worktree_selects_the_profile_for_the_repository() {
    let sandbox = with_other_profile();
    assert_committed(&run(&mut sandbox.commit(&["-m", "initial"]), ""));
    sandbox.git_ok(&["worktree", "add", "--quiet", "../linked"]);

    let output = run(
        sandbox
            .command(&["self", "setup", "other"])
            .current_dir(sandbox.path("linked")),
        "",
    );

    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_setup_output(
        &sandbox,
        "linked",
        &text(&output.stdout),
        &["profile: other", "commit hook: not installed"],
    );
    assert_eq!(configured_keys(&sandbox), [selection("other")]);
}

#[test]
fn setup_with_an_undefined_profile_writes_nothing() {
    for mode in SETUP_MODES {
        let sandbox = unconfigured();

        let output = run(&mut sandbox.command(&setup_command("work", mode)), "");

        assert_eq!(output.status.code(), Some(1), "{mode:?}");
        assert_eq!(
            text(&output.stderr),
            format!(
                "agent-gh: {} does not define profile `work`; defined profiles: test\n",
                sandbox.path("config.toml").display()
            )
        );
        assert_eq!(configured_keys(&sandbox), Vec::<String>::new(), "{mode:?}");
    }
}

#[test]
fn setup_without_configuration_writes_nothing() {
    for mode in SETUP_MODES {
        let sandbox = Sandbox::new();

        let output = run(&mut sandbox.command(&setup_command("test", mode)), "");

        assert_eq!(output.status.code(), Some(1), "{mode:?}");
        let stderr = text(&output.stderr);
        assert!(
            stderr.contains(&sandbox.path("config.toml").display().to_string()),
            "{mode:?}: {stderr}"
        );
        assert_eq!(configured_keys(&sandbox), Vec::<String>::new(), "{mode:?}");
    }
}

#[test]
fn setup_outside_a_repository_fails() {
    for mode in SETUP_MODES {
        let sandbox = unconfigured();

        let output = run(
            sandbox
                .command(&setup_command("test", mode))
                .current_dir(sandbox.path("outside")),
            "",
        );

        assert_eq!(output.status.code(), Some(1), "{mode:?}");
        let stderr = text(&output.stderr);
        assert!(
            stderr.starts_with("agent-gh: setup must run in a Git working tree: "),
            "{mode:?}: {stderr}"
        );
    }
}

#[test]
fn hook_installation_requires_a_token_and_an_identity() {
    for (args, selected) in [
        (&["self", "setup", "test", "--git-hooks"][..], "other"),
        (&["self", "install-git-hooks"], "test"),
    ] {
        for cache in ["token-1-2.toml", "identity-1.toml"] {
            let sandbox = with_other_profile();
            sandbox.select_profile(selected);
            fs_err::remove_file(sandbox.path(&format!("cache/{cache}")))
                .expect("cache file is removed");

            let output = run(&mut sandbox.command(args), "");

            assert_eq!(output.status.code(), Some(1), "{args:?} without {cache}");
            let stderr = text(&output.stderr);
            assert!(
                stderr.contains("missing.pem"),
                "{args:?} without {cache}: {stderr}"
            );
            assert_eq!(
                configured_keys(&sandbox),
                [selection(selected)],
                "{args:?} without {cache}"
            );
        }
    }
}

#[test]
fn install_git_hooks_uses_the_selected_profile() {
    let sandbox = with_other_profile();
    sandbox.setup(&["other"]);

    let output = run(&mut sandbox.command(&["self", "install-git-hooks"]), "");

    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_setup_output(
        &sandbox,
        "work",
        &text(&output.stdout),
        &[
            "profile: other",
            "commit hook: installed",
            &format!("co-author: {OTHER_CO_AUTHOR}"),
        ],
    );
    assert_eq!(configured_keys(&sandbox), expected_keys("other"));
    assert_committed(&sandbox.agent_commit(&["-m", "subject"]));
    assert_eq!(co_authors(&sandbox.last_message()), [OTHER_CO_AUTHOR]);
}

#[test]
fn install_git_hooks_without_a_selected_profile_writes_nothing() {
    let sandbox = unconfigured();

    let output = run(&mut sandbox.command(&["self", "install-git-hooks"]), "");

    assert_eq!(output.status.code(), Some(1));
    let stderr = text(&output.stderr);
    let Some((root, rest)) = stderr
        .strip_prefix("agent-gh: no profile is selected for the repository at ")
        .and_then(|rest| rest.split_once(", so agent-gh did not install the commit hook.\n"))
    else {
        panic!("unexpected message: {stderr}");
    };
    assert!(same_path(root, &sandbox.path("work")), "{stderr}");
    assert!(
        rest.starts_with(
            "Ask the user to run `agent-gh self setup <profile>` in that repository. "
        ),
        "{stderr}"
    );
    assert_eq!(configured_keys(&sandbox), Vec::<String>::new());
}

#[test]
fn install_git_hooks_outside_a_repository_fails() {
    let sandbox = unconfigured();

    let output = run(
        sandbox
            .command(&["self", "install-git-hooks"])
            .current_dir(sandbox.path("outside")),
        "",
    );

    assert_eq!(output.status.code(), Some(1));
    let stderr = text(&output.stderr);
    assert!(
        stderr.starts_with("agent-gh: install-git-hooks must run in a Git working tree: "),
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
fn remove_git_hooks_is_repeatable_and_keeps_the_profile_and_unrelated_hooks() {
    let sandbox = Sandbox::with_hooks();
    add_unrelated_hook(&sandbox);

    for attempt in 1..=2 {
        let output = run(&mut sandbox.command(&["self", "remove-git-hooks"]), "");

        assert!(
            output.status.success(),
            "attempt {attempt}: {}",
            text(&output.stderr)
        );
        assert_eq!(
            text(&output.stdout),
            "removed the commit hook; kept the profile selection\n"
        );
        assert_eq!(
            configured_keys(&sandbox),
            sorted(iter::once(selection("test")).chain(unrelated_keys())),
            "attempt {attempt}"
        );
    }
    assert_committed(&sandbox.agent_commit(&["-m", "subject"]));
    assert_eq!(sandbox.last_message(), "subject\n\n");
    sandbox.write_config_with(ISSUE_LIST_AS_BOT);
    assert_issue_list_uses_the_profile(&sandbox);
}

#[test]
fn hook_free_setup_supports_proxy_and_refresh() {
    let sandbox = Sandbox::new();
    sandbox.write_config_with(ISSUE_LIST_AS_BOT);
    sandbox.seed_token();
    sandbox.install_fake_gh();

    sandbox.setup(&["test"]);

    assert_issue_list_uses_the_profile(&sandbox);
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

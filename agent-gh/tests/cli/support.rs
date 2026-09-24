use jiff::SignedDuration;
use jiff::Timestamp;
use serde_json::Value;
use std::env;
use std::env::consts::EXE_SUFFIX;
use std::ffi::OsString;
use std::io::Write;
use std::iter;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;
use tempfile::TempDir;

pub(crate) const CACHED_TOKEN: &str = "cached-installation-token";
pub(crate) const CO_AUTHOR: &str = "test-app[bot] <42+test-app[bot]@users.noreply.github.com>";
pub(crate) const AGENT_MARKER: &str = "CLAUDE_CODE_CHILD_SESSION";

const AGENT_VARIABLES: &[&str] = &[
    "CLAUDE_CODE_CHILD_SESSION",
    "CODEX_THREAD_ID",
    "GEMINI_CLI",
    "COPILOT_CLI",
    "CURSOR_AGENT",
    "CLAUDECODE",
];
const GIT_VARIABLES: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_COMMON_DIR",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
];
const GLOBAL_GIT_CONFIG: &str = "\
[user]
\tname = Test User
\temail = test@example.com
[init]
\tdefaultBranch = main
[commit]
\tgpgSign = false
";

pub(crate) struct Sandbox {
    root: TempDir,
}

impl Sandbox {
    pub(crate) fn new() -> Self {
        let root = tempfile::tempdir().expect("temporary directory is created");
        for dir in ["bin", "cache", "outside", "work"] {
            fs_err::create_dir(root.path().join(dir)).expect("sandbox directory is created");
        }
        fs_err::write(root.path().join("gitconfig"), GLOBAL_GIT_CONFIG)
            .expect("global Git config is written");
        let sandbox = Self { root };
        sandbox.git_ok(&["init", "--quiet"]);
        sandbox
    }

    pub(crate) fn configured() -> Self {
        let sandbox = Self::new();
        sandbox.write_config();
        sandbox.select_profile("test");
        sandbox.seed_token();
        sandbox.seed_identity();
        sandbox.install_fake_gh();
        sandbox
    }

    pub(crate) fn with_hooks() -> Self {
        let sandbox = Self::configured();
        sandbox.setup_git_hooks("test");
        sandbox
    }

    pub(crate) fn setup_git_hooks(&self, profile: &str) -> String {
        let output = run(&mut self.command(&["self", "setup-git-hooks", profile]), "");
        assert!(
            output.status.success(),
            "setup-git-hooks failed: {}",
            text(&output.stderr)
        );
        text(&output.stdout)
    }

    pub(crate) fn path(&self, relative: &str) -> PathBuf {
        self.root.path().join(relative)
    }

    pub(crate) fn write_config(&self) {
        self.write_config_with("");
    }

    pub(crate) fn write_config_with(&self, extra_lines: &str) {
        fs_err::write(
            self.path("config.toml"),
            format!(
                "[profiles.test]\napp_id = 1\ninstallation_id = 2\nprivate_key_path = \"missing.pem\"\n{extra_lines}"
            ),
        )
        .expect("configuration is written");
    }

    pub(crate) fn select_profile(&self, name: &str) {
        self.git_ok(&["config", "--local", "agent-gh.profile", name]);
    }

    pub(crate) fn seed_token(&self) {
        self.seed_token_for(1, 2);
    }

    pub(crate) fn seed_token_for(&self, app_id: u64, installation_id: u64) {
        let expires_at = Timestamp::now()
            .checked_add(SignedDuration::from_secs(60 * 60))
            .expect("expiry is representable");
        fs_err::write(
            self.path(&format!("cache/token-{app_id}-{installation_id}.toml")),
            format!(
                "app_id = {app_id}\ninstallation_id = {installation_id}\ntoken = \"{CACHED_TOKEN}\"\nexpires_at = \"{expires_at}\"\n"
            ),
        )
        .expect("token cache is written");
    }

    pub(crate) fn seed_identity(&self) {
        self.seed_identity_for(1, "test-app", 42);
    }

    pub(crate) fn seed_identity_for(&self, app_id: u64, slug: &str, bot_user_id: u64) {
        fs_err::write(
            self.path(&format!("cache/identity-{app_id}.toml")),
            format!("app_id = {app_id}\nslug = \"{slug}\"\nbot_user_id = {bot_user_id}\n"),
        )
        .expect("identity cache is written");
    }

    pub(crate) fn install_fake_gh(&self) {
        let source = agent_gh_dir()
            .join("examples")
            .join(format!("fake_gh{EXE_SUFFIX}"));
        assert!(
            source.is_file(),
            "{} is missing; build it with `cargo +stable build -p agent-gh --example fake_gh --locked`",
            source.display()
        );
        fs_err::copy(&source, self.path(&format!("bin/gh{EXE_SUFFIX}")))
            .expect("fake gh is copied");
    }

    pub(crate) fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_agent-gh"));
        command
            .args(args)
            .env("FAKE_GH_RECORD", self.path("record.json"))
            .env_remove("FAKE_GH_EXIT");
        self.isolate(&mut command);
        command
    }

    pub(crate) fn git(&self, args: &[&str]) -> Command {
        let mut command = Command::new("git");
        command.args(args);
        self.isolate(&mut command);
        command
    }

    pub(crate) fn git_ok(&self, args: &[&str]) -> String {
        let output = run(&mut self.git(args), "");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            text(&output.stderr)
        );
        text(&output.stdout)
    }

    pub(crate) fn commit(&self, args: &[&str]) -> Command {
        self.git(&[&["commit", "--quiet", "--allow-empty"][..], args].concat())
    }

    pub(crate) fn agent_commit(&self, args: &[&str]) -> Output {
        run(self.commit(args).env(AGENT_MARKER, "1"), "")
    }

    pub(crate) fn last_message(&self) -> String {
        self.git_ok(&["log", "-1", "--format=%B"])
    }

    pub(crate) fn path_without_agent_gh(&self) -> OsString {
        join_paths(iter::once(self.path("bin")).chain(inherited_path_without("agent-gh")))
    }

    pub(crate) fn path_without_gh() -> OsString {
        let exec_path = Command::new("git")
            .arg("--exec-path")
            .output()
            .expect("git runs");
        join_paths([PathBuf::from(text(&exec_path.stdout).trim())])
    }

    pub(crate) fn record(&self) -> Option<Value> {
        let path = self.path("record.json");
        path.exists().then(|| {
            let text = fs_err::read_to_string(path).expect("record is readable");
            serde_json::from_str(&text).expect("record is JSON")
        })
    }

    fn isolate(&self, command: &mut Command) {
        let search_path = join_paths(
            [self.path("bin"), agent_gh_dir()]
                .into_iter()
                .chain(inherited_path_without("agent-gh")),
        );
        command
            .current_dir(self.path("work"))
            .env("AGENT_GH_CONFIG", self.path("config.toml"))
            .env("AGENT_GH_CACHE_DIR", self.path("cache"))
            .env("PATH", search_path)
            .env("GIT_CONFIG_GLOBAL", self.path("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("LC_ALL", "C")
            .env(
                "GIT_CEILING_DIRECTORIES",
                self.root.path().parent().expect("sandbox has a parent"),
            );
        for name in AGENT_VARIABLES.iter().chain(GIT_VARIABLES) {
            command.env_remove(name);
        }
    }
}

pub(crate) fn run(command: &mut Command, stdin: &str) -> Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("command starts");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(stdin.as_bytes())
        .expect("stdin is written");
    child.wait_with_output().expect("command exits")
}

pub(crate) fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).expect("output is UTF-8")
}

pub(crate) fn value<'a>(lines: &'a [String], key: &str) -> &'a str {
    let prefix = format!("{key}: ");
    lines
        .iter()
        .find_map(|line| line.strip_prefix(&prefix))
        .unwrap_or_else(|| panic!("no `{key}` line in {lines:#?}"))
}

pub(crate) fn same_path(actual: &str, expected: &Path) -> bool {
    fs_err::canonicalize(actual).expect("actual path exists")
        == fs_err::canonicalize(expected).expect("expected path exists")
}

fn agent_gh_dir() -> PathBuf {
    Path::new(env!("CARGO_BIN_EXE_agent-gh"))
        .parent()
        .expect("agent-gh has a parent directory")
        .to_path_buf()
}

fn inherited_path_without(program: &str) -> Vec<PathBuf> {
    let names = [program.to_owned(), format!("{program}{EXE_SUFFIX}")];
    env::split_paths(&env::var_os("PATH").unwrap_or_default())
        .filter(|dir| !names.iter().any(|name| dir.join(name).is_file()))
        .collect()
}

fn join_paths(paths: impl IntoIterator<Item = PathBuf>) -> OsString {
    env::join_paths(paths).expect("PATH entries join")
}

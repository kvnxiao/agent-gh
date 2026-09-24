use jiff::SignedDuration;
use jiff::Timestamp;
use serde_json::Value;
use std::env;
use std::env::consts::EXE_SUFFIX;
use std::io::Write;
use std::iter;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;
use tempfile::TempDir;

pub(crate) const CACHED_TOKEN: &str = "cached-installation-token";

pub(crate) struct Sandbox {
    root: TempDir,
}

impl Sandbox {
    pub(crate) fn new() -> Self {
        let root = tempfile::tempdir().expect("temporary directory is created");
        for dir in ["bin", "cache", "work"] {
            fs_err::create_dir(root.path().join(dir)).expect("sandbox directory is created");
        }
        Self { root }
    }

    pub(crate) fn with_cached_token() -> Self {
        let sandbox = Self::new();
        sandbox.write_config();
        sandbox.seed_token();
        sandbox.install_fake_gh();
        sandbox
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
                "app_id = 1\ninstallation_id = 2\nprivate_key_path = \"missing.pem\"\n{extra_lines}"
            ),
        )
        .expect("configuration is written");
    }

    pub(crate) fn seed_token(&self) {
        let expires_at = Timestamp::now()
            .checked_add(SignedDuration::from_secs(60 * 60))
            .expect("expiry is representable");
        fs_err::write(
            self.path("cache/token.toml"),
            format!(
                "app_id = 1\ninstallation_id = 2\ntoken = \"{CACHED_TOKEN}\"\nexpires_at = \"{expires_at}\"\n"
            ),
        )
        .expect("token cache is written");
    }

    pub(crate) fn install_fake_gh(&self) {
        let source = Path::new(env!("CARGO_BIN_EXE_agent-gh"))
            .with_file_name("examples")
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
        let search_path = env::join_paths(
            iter::once(self.path("bin"))
                .chain(env::split_paths(&env::var_os("PATH").unwrap_or_default())),
        )
        .expect("PATH entries join");
        let mut command = Command::new(env!("CARGO_BIN_EXE_agent-gh"));
        command
            .args(args)
            .current_dir(self.path("work"))
            .env("AGENT_GH_CONFIG", self.path("config.toml"))
            .env("AGENT_GH_CACHE_DIR", self.path("cache"))
            .env("FAKE_GH_RECORD", self.path("record.json"))
            .env("PATH", search_path)
            .env_remove("FAKE_GH_EXIT");
        command
    }

    pub(crate) fn record(&self) -> Option<Value> {
        let path = self.path("record.json");
        path.exists().then(|| {
            let text = fs_err::read_to_string(path).expect("record is readable");
            serde_json::from_str(&text).expect("record is JSON")
        })
    }
}

pub(crate) fn run(command: &mut Command, stdin: &str) -> Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("agent-gh starts");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(stdin.as_bytes())
        .expect("stdin is written");
    child.wait_with_output().expect("agent-gh exits")
}

pub(crate) fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).expect("output is UTF-8")
}

use crate::cache;
use crate::config::Config;
use crate::config::Paths;
use crate::github::GitHub;
use crate::hook;
use crate::proxy;
use anyhow::Result;
use jiff::Timestamp;
use std::borrow::Cow;
use std::ffi::OsString;
use std::fmt::Display;
use std::io;
use std::io::Write;
use std::process::ExitCode;

const USAGE: &str = "\
Usage: agent-gh <gh arguments>...
       agent-gh self <command>

Run the GitHub CLI with a GitHub App installation token. Commands that start
with a run_as_user entry in the configuration file run with the user's
credentials instead.

Wrapper commands:
  self --help       Print this usage
  self --version    Print the agent-gh version
  self status       Print the configuration, cache path, and cached token expiry
  self refresh      Request a new installation token and replace the cache
  self hook-check   Check a Claude Code or Codex PreToolUse payload on stdin

Environment:
  AGENT_GH_CONFIG      Configuration file (default: ~/.config/agent-gh/config.toml)
  AGENT_GH_CACHE_DIR   Token cache directory (default: <cache dir>/agent-gh)
";

const VERSION: &str = concat!(env!("CARGO_PKG_NAME"), " ", env!("CARGO_PKG_VERSION"), "\n");
const USAGE_ERROR: u8 = 2;
const HOOK_BLOCK: u8 = 2;

pub(crate) fn run(args: &[OsString]) -> ExitCode {
    match args.split_first() {
        None => print_stdout(USAGE),
        Some((first, rest)) if first == "self" => run_self(rest),
        Some(_) => report(proxy_gh(args)),
    }
}

fn run_self(args: &[OsString]) -> ExitCode {
    let args: Vec<Cow<'_, str>> = args.iter().map(|arg| arg.to_string_lossy()).collect();
    let args: Vec<&str> = args.iter().map(AsRef::as_ref).collect();
    match args.as_slice() {
        [] | ["--help" | "-h"] => print_stdout(USAGE),
        ["--version"] => print_stdout(VERSION),
        ["status"] => report(status()),
        ["refresh"] => report(refresh()),
        ["hook-check"] => hook_check(),
        _ => fail(
            format_args!(
                "unknown wrapper command `self {}`; run `agent-gh self --help`",
                args.join(" ")
            ),
            ExitCode::from(USAGE_ERROR),
        ),
    }
}

fn proxy_gh(args: &[OsString]) -> Result<ExitCode> {
    let paths = Paths::from_env()?;
    let config = Config::load(&paths.config)?;
    if config.run_as_user.iter().any(|prefix| prefix.matches(args)) {
        return proxy::run_gh_as_user(args);
    }
    let token = cache::obtain(&config, &paths.cache, &GitHub::new(), Timestamp::now())?;
    proxy::run_gh(args, &token.token)
}

fn status() -> Result<ExitCode> {
    let paths = Paths::from_env()?;
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "config: {}", paths.config)?;
    writeln!(stdout, "cache: {}", paths.cache)?;
    let config = Config::load(&paths.config)?;
    writeln!(stdout, "app_id: {}", config.app_id)?;
    writeln!(stdout, "installation_id: {}", config.installation_id)?;
    writeln!(stdout, "private_key_path: {}", config.private_key_path)?;
    let run_as_user: Vec<String> = config
        .run_as_user
        .iter()
        .map(|prefix| format!("\"{prefix}\""))
        .collect();
    writeln!(stdout, "run_as_user: [{}]", run_as_user.join(", "))?;
    let cached = cache::read(&paths.cache)?;
    let state = cache::describe(cached.as_ref(), &config, Timestamp::now());
    writeln!(stdout, "token: {state}")?;
    Ok(ExitCode::SUCCESS)
}

fn refresh() -> Result<ExitCode> {
    let paths = Paths::from_env()?;
    let config = Config::load(&paths.config)?;
    let token = cache::refresh(&config, &paths.cache, &GitHub::new(), Timestamp::now())?;
    writeln!(
        io::stdout().lock(),
        "token refreshed; expires at {}",
        token.expires_at
    )?;
    Ok(ExitCode::SUCCESS)
}

fn hook_check() -> ExitCode {
    let Some(message) = hook::block_message(io::stdin().lock()) else {
        return ExitCode::SUCCESS;
    };
    // The harness blocks the command on exit status 2 even when the stderr
    // write fails.
    match writeln!(io::stderr().lock(), "{message}") {
        Ok(()) | Err(_) => ExitCode::from(HOOK_BLOCK),
    }
}

fn report(result: Result<ExitCode>) -> ExitCode {
    match result {
        Ok(code) => code,
        Err(error) => fail(format_args!("{error:#}"), ExitCode::FAILURE),
    }
}

fn print_stdout(text: &str) -> ExitCode {
    match io::stdout().lock().write_all(text.as_bytes()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => fail(
            format_args!("writing to stdout: {error}"),
            ExitCode::FAILURE,
        ),
    }
}

fn fail(message: impl Display, code: ExitCode) -> ExitCode {
    match writeln!(io::stderr().lock(), "agent-gh: {message}") {
        Ok(()) => code,
        Err(_) => ExitCode::FAILURE,
    }
}

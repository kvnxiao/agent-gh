use crate::cache;
use crate::co_author;
use crate::config::Config;
use crate::config::Paths;
use crate::config::Profile;
use crate::git;
use crate::git::ProfileLookup;
use crate::git::Worktree;
use crate::github::GitHub;
use crate::hook;
use crate::identity;
use crate::proxy;
use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use camino::Utf8Path;
use camino::Utf8PathBuf;
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

Run the GitHub CLI with the user's normal gh authentication. A command that
matches the run_as_bot rules of the selected profile runs with the
installation token of the profile's GitHub App instead. The Git config of the
repository in the working directory selects the profile from the
configuration file.

Wrapper commands:
  self --help                     Print this usage
  self --version                  Print the agent-gh version
  self status                     Print the configuration, profile, and caches
  self refresh                    Replace the cached token and App identity
  self setup <profile>            Select a profile for the repository
  self setup <profile> --git-hooks
                                  Select a profile and install the commit hook
  self install-git-hooks          Install the commit hook for the selected
                                  profile
  self remove-git-hooks           Remove the commit hook and keep the profile
  self co-author                  Print the commit hook's co-author value
  self hook-check                 Check a Claude Code or Codex PreToolUse
                                  payload on stdin

Environment:
  AGENT_GH_CONFIG      Configuration file (default: ~/.config/agent-gh/config.toml)
  AGENT_GH_CACHE_DIR   Cache directory (default: <cache dir>/agent-gh)
";

const VERSION: &str = concat!(env!("CARGO_PKG_NAME"), " ", env!("CARGO_PKG_VERSION"), "\n");
const USAGE_ERROR: u8 = 2;
const HOOK_BLOCK: u8 = 2;
const SETUP_COMMAND: &str = "agent-gh self setup <profile>";
const SETUP_USAGE: &str = "agent-gh self setup <profile> [--git-hooks]";
const GIT_HOOKS_FLAG: &str = "--git-hooks";

#[derive(Clone, Copy, PartialEq, Eq)]
enum GitHooks {
    Keep,
    Install,
}

enum Selection<'a> {
    Profile(&'a Profile),
    Unselected(Utf8PathBuf),
    NoRepository(String),
}

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
        ["setup", rest @ ..] => match setup_args(rest) {
            Some((profile, hooks)) => report(setup(profile, hooks)),
            None => usage_error(SETUP_USAGE),
        },
        ["install-git-hooks"] => report(install_git_hooks()),
        ["install-git-hooks", ..] => usage_error("agent-gh self install-git-hooks"),
        ["remove-git-hooks"] => report(remove_git_hooks()),
        ["remove-git-hooks", ..] => usage_error("agent-gh self remove-git-hooks"),
        ["co-author"] => co_author::run(co_author_value),
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
    let profile = require_profile(&config, "run gh")?;
    if !profile.run_as_bot.selects_bot(args) {
        return proxy::run_gh_as_user(args);
    }
    let token = cache::obtain(profile, &paths.cache_dir, &GitHub::new(), Timestamp::now())?;
    proxy::run_gh(args, &token.token)
}

fn select(config: &Config) -> Result<Selection<'_>> {
    Ok(match git::profile_lookup()? {
        ProfileLookup::Selected(name) => Selection::Profile(
            config
                .profile(&name)
                .context("the repository selects an undefined profile")?,
        ),
        ProfileLookup::Unselected => match git::worktree()? {
            Worktree::Root(root) => Selection::Unselected(root),
            Worktree::NoRepository(stderr) => Selection::NoRepository(stderr),
        },
        ProfileLookup::NoRepository(stderr) => Selection::NoRepository(stderr),
    })
}

fn require_profile<'a>(config: &'a Config, action: &str) -> Result<&'a Profile> {
    let profiles = || format!("Profiles in {}: {}", config.path, config.profile_names());
    match select(config)? {
        Selection::Profile(profile) => Ok(profile),
        Selection::Unselected(root) => bail!(
            "no profile is selected for the repository at {root}, so agent-gh did not {action}.\n\
             Ask the user to run `{SETUP_COMMAND}` in that repository. {}",
            profiles()
        ),
        Selection::NoRepository(stderr) => bail!(
            "the profile comes from the Git repository in the working directory, and Git found \
             no usable repository there, so agent-gh did not {action}. Git reported:\n{stderr}\n\
             Run the command in a repository where the user ran `{SETUP_COMMAND}`. {}",
            profiles()
        ),
    }
}

fn co_author_value() -> Result<String> {
    let paths = Paths::from_env()?;
    let config = Config::load(&paths.config)?;
    let profile = match select(&config)? {
        Selection::Profile(profile) => profile,
        Selection::Unselected(root) => {
            bail!("no profile is selected for the repository at {root}")
        }
        Selection::NoRepository(stderr) => {
            bail!("Git found no usable repository in the working directory: {stderr}")
        }
    };
    let identity = identity::obtain(profile, &paths.cache_dir, &GitHub::new(), Timestamp::now())?;
    Ok(identity.co_author())
}

fn status() -> Result<ExitCode> {
    let paths = Paths::from_env()?;
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "config: {}", paths.config)?;
    let config = Config::load(&paths.config)?;
    writeln!(stdout, "profiles: {}", config.profile_names())?;
    let in_repository = match git::worktree()? {
        Worktree::Root(root) => {
            writeln!(stdout, "repository: {root}")?;
            true
        }
        Worktree::NoRepository(stderr) => {
            let cause = stderr.lines().next().unwrap_or_default();
            writeln!(stdout, "repository: none ({cause})")?;
            false
        }
    };
    match git::profile_lookup()? {
        ProfileLookup::Selected(name) => match config.profiles.get(&name) {
            Some(profile) => {
                writeln!(stdout, "profile: {name}")?;
                write_profile(&mut stdout, profile, &paths.cache_dir)?;
            }
            None => writeln!(
                stdout,
                "profile: {name}, which {} does not define",
                config.path
            )?,
        },
        ProfileLookup::Unselected | ProfileLookup::NoRepository(_) => writeln!(
            stdout,
            "profile: none; run `{SETUP_COMMAND}` in the repository"
        )?,
    }
    if in_repository {
        writeln!(stdout, "commit hook: {}", hook_state()?)?;
    }
    Ok(ExitCode::SUCCESS)
}

fn write_profile(stdout: &mut impl Write, profile: &Profile, cache_dir: &Utf8Path) -> Result<()> {
    writeln!(stdout, "app_id: {}", profile.app_id)?;
    writeln!(stdout, "installation_id: {}", profile.installation_id)?;
    writeln!(stdout, "private_key_path: {}", profile.private_key_path)?;
    writeln!(stdout, "run_as_bot: {}", profile.run_as_bot)?;
    writeln!(
        stdout,
        "token cache: {}",
        cache::token_path(cache_dir, profile)
    )?;
    let token = cache::read(cache_dir, profile)?;
    let state = cache::describe(token.as_ref(), profile, Timestamp::now());
    writeln!(stdout, "token: {state}")?;
    writeln!(
        stdout,
        "identity cache: {}",
        identity::path(cache_dir, profile)
    )?;
    let cached = identity::read(cache_dir, profile)?;
    writeln!(stdout, "co-author: {}", identity::describe(cached.as_ref()))?;
    Ok(())
}

fn refresh() -> Result<ExitCode> {
    let paths = Paths::from_env()?;
    let config = Config::load(&paths.config)?;
    let profile = require_profile(&config, "refresh the caches")?;
    let github = GitHub::new();
    let now = Timestamp::now();
    let token = cache::refresh(profile, &paths.cache_dir, &github, now)?;
    let identity = identity::refresh(profile, &paths.cache_dir, &github, now)?;
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "token refreshed; expires at {}", token.expires_at)?;
    writeln!(stdout, "co-author: {}", identity.co_author())?;
    Ok(ExitCode::SUCCESS)
}

fn setup_args<'a>(args: &[&'a str]) -> Option<(&'a str, GitHooks)> {
    let (profile, hooks) = match *args {
        [profile] => (profile, GitHooks::Keep),
        [GIT_HOOKS_FLAG, profile] | [profile, GIT_HOOKS_FLAG] => (profile, GitHooks::Install),
        _ => return None,
    };
    (!profile.starts_with('-')).then_some((profile, hooks))
}

fn setup(name: &str, hooks: GitHooks) -> Result<ExitCode> {
    let paths = Paths::from_env()?;
    let config = Config::load(&paths.config)?;
    let profile = config.profile(name)?;
    let (root, co_author) = match hooks {
        GitHooks::Keep => (require_worktree("setup")?, None),
        GitHooks::Install => {
            let root = require_hook_support("setup")?;
            (root, Some(hook_co_author(profile, &paths)?))
        }
    };
    git::select_profile(&profile.name)?;
    if hooks == GitHooks::Install {
        git::install_hooks()?;
    }
    print_setup(&root, profile, co_author.as_deref())
}

fn install_git_hooks() -> Result<ExitCode> {
    let paths = Paths::from_env()?;
    let config = Config::load(&paths.config)?;
    let root = require_hook_support("install-git-hooks")?;
    let profile = require_profile(&config, "install the commit hook")?;
    let co_author = hook_co_author(profile, &paths)?;
    git::install_hooks()?;
    print_setup(&root, profile, Some(&co_author))
}

fn require_hook_support(command: &str) -> Result<Utf8PathBuf> {
    git::require_config_hooks()?;
    require_worktree(command)
}

fn require_worktree(command: &str) -> Result<Utf8PathBuf> {
    match git::worktree()? {
        Worktree::Root(root) => Ok(root),
        Worktree::NoRepository(stderr) => {
            bail!("{command} must run in a Git working tree: {stderr}")
        }
    }
}

fn hook_co_author(profile: &Profile, paths: &Paths) -> Result<String> {
    let github = GitHub::new();
    let now = Timestamp::now();
    cache::obtain(profile, &paths.cache_dir, &github, now)?;
    let identity = identity::obtain(profile, &paths.cache_dir, &github, now)?;
    Ok(identity.co_author())
}

fn print_setup(root: &Utf8Path, profile: &Profile, co_author: Option<&str>) -> Result<ExitCode> {
    let hook_state = hook_state()?;
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "repository: {root}")?;
    writeln!(stdout, "profile: {}", profile.name)?;
    writeln!(stdout, "commit hook: {hook_state}")?;
    if let Some(co_author) = co_author {
        writeln!(stdout, "co-author: {co_author}")?;
    }
    Ok(ExitCode::SUCCESS)
}

fn hook_state() -> Result<&'static str> {
    Ok(if git::hook_installed()? {
        "installed"
    } else {
        "not installed"
    })
}

fn remove_git_hooks() -> Result<ExitCode> {
    git::remove_hooks()?;
    writeln!(
        io::stdout().lock(),
        "removed the commit hook; kept the profile selection"
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

fn usage_error(usage: &str) -> ExitCode {
    fail(format_args!("usage: {usage}"), ExitCode::from(USAGE_ERROR))
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

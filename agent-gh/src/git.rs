use anyhow::Context;
use anyhow::Result;
use anyhow::anyhow;
use anyhow::bail;
use camino::Utf8PathBuf;
use std::fmt;
use std::io;
use std::iter;
use std::process::Command;

const PROFILE_KEY: &str = "agent-gh.profile";
const HOOK_COMMAND_KEY: &str = "hook.agent-gh.commit-msg.command";
const HOOK_KEYS: &[(&str, &str)] = &[
    ("hook.agent-gh.commit-msg.event", "commit-msg"),
    (
        HOOK_COMMAND_KEY,
        r#"git interpret-trailers --in-place --trim-empty --if-exists addIfDifferent --trailer "Co-authored-by: $(agent-gh self co-author)""#,
    ),
];
const CONFIG_HOOKS_VERSION: Version = Version {
    major: 2,
    minor: 54,
};
const GET_KEY_ABSENT: i32 = 1;
const UNSET_KEY_ABSENT: i32 = 5;

pub(crate) enum ProfileLookup {
    Selected(String),
    Unselected,
    NoRepository(String),
}

pub(crate) enum Worktree {
    Root(Utf8PathBuf),
    NoRepository(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Version {
    major: u32,
    minor: u32,
}

impl fmt::Display for Version {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}", self.major, self.minor)
    }
}

struct Output {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl Output {
    fn succeeded(&self) -> bool {
        self.code == Some(0)
    }
}

pub(crate) fn profile_lookup() -> Result<ProfileLookup> {
    let output = git(&["config", "--local", "--get", PROFILE_KEY])?;
    Ok(match output.code {
        Some(0) => ProfileLookup::Selected(output.stdout.trim().to_owned()),
        Some(GET_KEY_ABSENT) => ProfileLookup::Unselected,
        _ => ProfileLookup::NoRepository(output.stderr),
    })
}

pub(crate) fn worktree() -> Result<Worktree> {
    let output = git(&["rev-parse", "--show-toplevel"])?;
    Ok(if output.succeeded() {
        Worktree::Root(Utf8PathBuf::from(output.stdout.trim()))
    } else {
        Worktree::NoRepository(output.stderr)
    })
}

pub(crate) fn hook_installed() -> Result<bool> {
    Ok(git(&["config", "--local", "--get", HOOK_COMMAND_KEY])?.succeeded())
}

pub(crate) fn require_config_hooks() -> Result<()> {
    let output = git(&["version"])?;
    if !output.succeeded() {
        bail!("`git version` failed: {}", output.stderr);
    }
    check_version(&output.stdout)
}

pub(crate) fn install_hooks(profile: &str) -> Result<()> {
    for &(key, value) in iter::once(&(PROFILE_KEY, profile)).chain(HOOK_KEYS) {
        let output = git(&["config", "--local", "--replace-all", key, value])?;
        if !output.succeeded() {
            bail!(
                "`git config --local --replace-all {key}` failed: {}",
                output.stderr
            );
        }
    }
    Ok(())
}

pub(crate) fn remove_hooks() -> Result<()> {
    for key in iter::once(PROFILE_KEY).chain(HOOK_KEYS.iter().map(|&(key, _)| key)) {
        let output = git(&["config", "--local", "--unset-all", key])?;
        if !matches!(output.code, Some(0 | UNSET_KEY_ABSENT)) {
            bail!(
                "`git config --local --unset-all {key}` failed: {}",
                output.stderr
            );
        }
    }
    Ok(())
}

fn check_version(output: &str) -> Result<()> {
    let output = output.trim();
    let version = parse_version(output)
        .with_context(|| format!("cannot read the Git version from {output:?}"))?;
    if version < CONFIG_HOOKS_VERSION {
        bail!("the commit hook requires Git {CONFIG_HOOKS_VERSION} or later; found {output:?}");
    }
    Ok(())
}

fn parse_version(output: &str) -> Option<Version> {
    let mut numbers = output.strip_prefix("git version ")?.split('.');
    let major = numbers.next()?.parse().ok()?;
    let minor = numbers.next()?.parse().ok()?;
    Some(Version { major, minor })
}

fn git(args: &[&str]) -> Result<Output> {
    let output = Command::new("git").args(args).output().map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            anyhow!("git was not found on PATH")
        } else {
            anyhow::Error::new(error).context("starting git")
        }
    })?;
    Ok(Output {
        code: output.status.code(),
        stdout: String::from_utf8(output.stdout)
            .with_context(|| format!("`git {}` printed non-UTF-8 output", args.join(" ")))?,
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version_error(output: &str) -> String {
        match check_version(output) {
            Ok(()) => panic!("{output:?} should be rejected"),
            Err(error) => format!("{error:#}"),
        }
    }

    #[test]
    fn accepts_git_with_config_hooks() {
        for output in [
            "git version 2.55.0.windows.3\n",
            "git version 2.54.0\n",
            "git version 3.0.0\n",
        ] {
            let result = check_version(output);
            assert!(result.is_ok(), "{output:?}: {result:?}");
        }
    }

    #[test]
    fn rejects_git_without_config_hooks() {
        assert_eq!(
            version_error("git version 2.53.1\n"),
            r#"the commit hook requires Git 2.54 or later; found "git version 2.53.1""#
        );
        assert_eq!(
            version_error("git version 2.39.5 (Apple Git-154)\n"),
            r#"the commit hook requires Git 2.54 or later; found "git version 2.39.5 (Apple Git-154)""#
        );
    }

    #[test]
    fn rejects_malformed_version_output() {
        for output in [
            "",
            "git version",
            "git version 2",
            "git version two.54",
            "2.54.0",
        ] {
            let message = version_error(output);
            assert!(
                message.starts_with("cannot read the Git version from"),
                "{output:?}: {message}"
            );
        }
    }
}

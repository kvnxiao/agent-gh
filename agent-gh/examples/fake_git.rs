//! Stand in for Git in `agent-gh` integration tests.
//!
//! The program runs the Git executable named by `FAKE_GIT_REAL` with its own
//! arguments, standard streams, working directory, and environment, and exits
//! with Git's status. When `FAKE_GIT_FAIL_KEY` names a config key, a
//! `git config --replace-all` or `git config --unset-all` invocation for that
//! key prints one line to stderr and exits 1 without running Git.

use anyhow::Context;
use anyhow::Result;
use std::env;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::io;
use std::io::Write;
use std::process::Command;
use std::process::ExitCode;

const WRITE_OPTIONS: &[&str] = &["--replace-all", "--unset-all"];

fn main() -> Result<ExitCode> {
    let args: Vec<OsString> = env::args_os().skip(1).collect();
    if let Some(key) = env::var_os("FAKE_GIT_FAIL_KEY")
        && writes_key(&args, &key)
    {
        writeln!(
            io::stderr().lock(),
            "fake git: refused to write {}",
            key.to_string_lossy()
        )?;
        return Ok(ExitCode::FAILURE);
    }
    let real = env::var_os("FAKE_GIT_REAL").context("FAKE_GIT_REAL is not set")?;
    let status = Command::new(&real)
        .args(&args)
        .status()
        .with_context(|| format!("running {}", real.to_string_lossy()))?;
    let code = status.code().context("git exited without a status code")?;
    Ok(ExitCode::from(u8::try_from(code).unwrap_or(u8::MAX)))
}

fn writes_key(args: &[OsString], key: &OsStr) -> bool {
    args.first().is_some_and(|command| command == "config")
        && args.iter().any(|arg| arg == key)
        && args
            .iter()
            .any(|arg| WRITE_OPTIONS.iter().any(|option| arg == option))
}

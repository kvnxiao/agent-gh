use anyhow::Result;
use anyhow::anyhow;
use std::ffi::OsString;
use std::io;
use std::process::Command;
use std::process::ExitCode;

const REMOVED_VARIABLES: &[&str] = &[
    "GITHUB_TOKEN",
    "GH_ENTERPRISE_TOKEN",
    "GITHUB_ENTERPRISE_TOKEN",
];

pub(crate) fn run_gh(args: &[OsString], token: &str) -> Result<ExitCode> {
    let mut command = Command::new("gh");
    command
        .args(args)
        .env("GH_TOKEN", token)
        .env("GH_HOST", "github.com");
    for name in REMOVED_VARIABLES {
        command.env_remove(name);
    }
    launch(command)
}

#[cfg(unix)]
fn launch(mut command: Command) -> Result<ExitCode> {
    use std::os::unix::process::CommandExt;

    Err(launch_error(command.exec()))
}

#[cfg(not(unix))]
fn launch(mut command: Command) -> Result<ExitCode> {
    let status = command.status().map_err(launch_error)?;
    Ok(status
        .code()
        .and_then(|code| u8::try_from(code).ok())
        .map_or(ExitCode::FAILURE, ExitCode::from))
}

fn launch_error(error: io::Error) -> anyhow::Error {
    if error.kind() == io::ErrorKind::NotFound {
        anyhow!("gh was not found on PATH")
    } else {
        anyhow::Error::new(error).context("starting gh")
    }
}

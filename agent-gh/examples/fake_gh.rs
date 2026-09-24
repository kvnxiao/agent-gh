//! Stand in for the GitHub CLI in `agent-gh` integration tests.
//!
//! The program writes its arguments, working directory, stdin, and credential
//! variables as JSON to the file named by `FAKE_GH_RECORD`, prints one line to
//! stdout and one to stderr, and exits with the status in `FAKE_GH_EXIT`
//! (default 0).

use anyhow::Context;
use anyhow::Result;
use serde_json::Map;
use serde_json::Value;
use std::env;
use std::io;
use std::io::Read;
use std::io::Write;
use std::process::ExitCode;

const RECORDED_VARIABLES: &[&str] = &[
    "GH_TOKEN",
    "GH_HOST",
    "GITHUB_TOKEN",
    "GH_ENTERPRISE_TOKEN",
    "GITHUB_ENTERPRISE_TOKEN",
];

fn main() -> Result<ExitCode> {
    let mut stdin = String::new();
    io::stdin()
        .read_to_string(&mut stdin)
        .context("reading stdin")?;
    let args: Vec<String> = env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let variables: Map<String, Value> = RECORDED_VARIABLES
        .iter()
        .map(|name| ((*name).to_owned(), Value::from(env::var(name).ok())))
        .collect();
    let record = serde_json::json!({
        "args": args,
        "cwd": env::current_dir().context("reading the working directory")?,
        "stdin": stdin,
        "env": variables,
    });
    let path = env::var_os("FAKE_GH_RECORD").context("FAKE_GH_RECORD is not set")?;
    fs_err::write(path, record.to_string())?;

    writeln!(io::stdout().lock(), "fake gh stdout")?;
    writeln!(io::stderr().lock(), "fake gh stderr")?;
    let status = match env::var("FAKE_GH_EXIT") {
        Ok(value) => value.parse().context("parsing FAKE_GH_EXIT")?,
        Err(_) => 0,
    };
    Ok(ExitCode::from(status))
}

//! Command-line entry point for `agent-gh`.

mod bash;
mod cache;
mod cli;
mod config;
mod github;
mod hook;
mod proxy;
#[cfg(test)]
mod test_support;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    cli::run(&args)
}

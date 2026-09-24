//! Command-line entry point for `agent-gh`.

mod bash;
mod cache;
mod cli;
mod co_author;
mod config;
mod git;
mod github;
mod hook;
mod identity;
mod proxy;
#[cfg(test)]
mod test_support;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    cli::run(&args)
}

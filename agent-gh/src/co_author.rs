use anyhow::Result;
use std::env;
use std::ffi::OsString;
use std::io;
use std::io::Write;
use std::process::ExitCode;

const AGENT_MARKERS: &[&str] = &[
    "CLAUDE_CODE_CHILD_SESSION",
    "CODEX_THREAD_ID",
    "GEMINI_CLI",
    "COPILOT_CLI",
    "CURSOR_AGENT",
];

pub(crate) fn run(co_author: impl FnOnce() -> Result<String>) -> ExitCode {
    if !is_agent(|name| env::var_os(name)) {
        return ExitCode::SUCCESS;
    }
    let written = match co_author() {
        Ok(value) => writeln!(io::stdout().lock(), "{value}"),
        Err(error) => writeln!(
            io::stderr().lock(),
            "agent-gh: {}; committed without the co-author trailer",
            one_line(&format!("{error:#}"))
        ),
    };
    match written {
        Ok(()) | Err(_) => ExitCode::SUCCESS,
    }
}

fn is_agent(lookup: impl Fn(&str) -> Option<OsString>) -> bool {
    AGENT_MARKERS
        .iter()
        .any(|name| lookup(name).is_some_and(|value| !value.is_empty()))
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn detects(variables: &[(&str, &str)]) -> bool {
        let variables: HashMap<&str, OsString> = variables
            .iter()
            .map(|(name, value)| (*name, OsString::from(value)))
            .collect();
        is_agent(|name| variables.get(name).cloned())
    }

    #[test]
    fn detects_each_agent_marker() {
        for &marker in AGENT_MARKERS {
            assert!(detects(&[(marker, "1")]), "{marker}");
        }
    }

    #[test]
    fn ignores_environments_without_a_non_empty_marker() {
        let empty_markers: Vec<(&str, &str)> =
            AGENT_MARKERS.iter().map(|&marker| (marker, "")).collect();
        for variables in [&[][..], &[("CLAUDECODE", "1")], &empty_markers] {
            assert!(!detects(variables), "{variables:?}");
        }
    }

    #[test]
    fn joins_multi_line_causes_into_one_line() {
        assert_eq!(
            one_line("parsing config.toml: TOML parse error\n  |\n1 | app_id = 1\n"),
            "parsing config.toml: TOML parse error | 1 | app_id = 1"
        );
    }
}

use crate::bash;
use crate::bash::MAX_NESTING;
use crate::bash::ParseError;
use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use serde::Deserialize;
use std::io::Read;

const MAX_PAYLOAD_BYTES: u64 = 1024 * 1024;

#[derive(Deserialize)]
struct Payload {
    tool_input: ToolInput,
}

#[derive(Deserialize)]
struct ToolInput {
    command: String,
}

pub(crate) fn block_message(payload: impl Read) -> Option<String> {
    let command = match read_command(payload) {
        Ok(command) => command,
        Err(error) => return Some(format!("agent-gh: cannot check this command: {error:#}")),
    };
    match bash::find_direct_gh(&command) {
        Ok(Some(word)) => Some(format!(
            "agent-gh: `{word}` runs the GitHub CLI with personal credentials. Replace `{word}` \
             with `agent-gh` to run the command as the GitHub App bot, or as the user when the \
             command matches the run_as_user configuration."
        )),
        Err(ParseError::TooDeep) => Some(format!(
            "agent-gh: cannot check this command: it nests more than {MAX_NESTING} levels of \
             substitutions"
        )),
        Ok(None) | Err(ParseError::Unparsable) => None,
    }
}

fn read_command(payload: impl Read) -> Result<String> {
    let mut reader = payload.take(MAX_PAYLOAD_BYTES + 1);
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .context("reading the hook payload")?;
    if reader.limit() == 0 {
        bail!("the hook payload exceeds {MAX_PAYLOAD_BYTES} bytes");
    }
    let payload: Payload = serde_json::from_slice(&bytes).context("parsing the hook payload")?;
    Ok(payload.tool_input.command)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(command: &str) -> String {
        serde_json::json!({
            "session_id": "session",
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_input": { "command": command, "description": "run" },
        })
        .to_string()
    }

    #[test]
    fn blocks_direct_gh_invocation() {
        let message =
            block_message(payload("gh issue list").as_bytes()).expect("command is blocked");
        assert!(
            message.contains("Replace `gh` with `agent-gh`"),
            "{message}"
        );
    }

    #[test]
    fn leaves_agent_gh_to_the_harness() {
        assert_eq!(
            block_message(payload("agent-gh issue list").as_bytes()),
            None
        );
    }

    #[test]
    fn leaves_unparsable_commands_to_the_harness() {
        assert_eq!(
            block_message(payload("gh issue list \"unclosed").as_bytes()),
            None
        );
    }

    #[test]
    fn blocks_commands_nested_beyond_the_parser_limit() {
        let command = format!("echo {}gh{}", "$(".repeat(10_000), ")".repeat(10_000));
        let message = block_message(payload(&command).as_bytes()).expect("command is blocked");
        assert!(message.contains("nests more than 64 levels"), "{message}");
    }

    #[test]
    fn blocks_invalid_json() {
        let message = block_message("{not json".as_bytes()).expect("payload is blocked");
        assert!(message.contains("parsing the hook payload"), "{message}");
    }

    #[test]
    fn blocks_payload_without_command() {
        let message = block_message(r#"{"tool_name":"Bash","tool_input":{}}"#.as_bytes())
            .expect("payload is blocked");
        assert!(message.contains("missing field `command`"), "{message}");
    }

    #[test]
    fn blocks_oversized_payload() {
        let oversized = payload(&"a".repeat(1024 * 1024));
        let message = block_message(oversized.as_bytes()).expect("payload is blocked");
        assert!(message.contains("exceeds 1048576 bytes"), "{message}");
    }

    #[test]
    fn accepts_payload_at_the_size_limit() {
        let base = payload("");
        let padding = usize::try_from(MAX_PAYLOAD_BYTES).expect("limit fits in usize") - base.len();
        let exact = payload(&" ".repeat(padding));
        assert_eq!(exact.len(), base.len() + padding);
        assert_eq!(block_message(exact.as_bytes()), None);
    }
}

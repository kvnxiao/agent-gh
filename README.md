# agent-gh

`agent-gh` runs the official GitHub CLI (`gh`) with a GitHub App installation token, so issues,
comments, and pull requests that an agent creates are attributed to the App's bot account instead
of the user. A hook in the agent harness (Claude Code or Codex) blocks direct `gh` commands and
tells the agent to use `agent-gh`.

## Setup

1. Create a GitHub App with the permissions the agent needs, such as Issues (read and write), Pull
   requests (read and write), and Contents (read). Install it on the repositories the agent works
   in; the installation's repository selection and permissions limit every token.
2. Download a private key for the App and record the App ID and the installation ID. The
   installation ID is the number at the end of the installation's settings URL.
3. Install the binary and write the configuration file:

   ```sh
   cargo install --locked --path agent-gh
   ```

   ```toml
   app_id = 1234567
   installation_id = 98765432
   private_key_path = "agent.private-key.pem"
   ```

   A relative `private_key_path` resolves against the configuration file's directory.

The configuration file is `~/.config/agent-gh/config.toml` on every platform, including Windows,
so one dotfiles layout covers all of them. When `XDG_CONFIG_HOME` is set to an absolute path, it
replaces `~/.config`. The token cache stays in the platform cache directory:

| Platform | Token cache                            |
| -------- | -------------------------------------- |
| Linux    | `~/.cache/agent-gh/token.toml`         |
| macOS    | `~/Library/Caches/agent-gh/token.toml` |
| Windows  | `%LOCALAPPDATA%\agent-gh\token.toml`   |

On Linux, `XDG_CACHE_HOME` replaces `~/.cache`. `AGENT_GH_CONFIG` names a different configuration
file, and `AGENT_GH_CACHE_DIR` names a different cache directory.

## Usage

`agent-gh` accepts the same arguments as `gh`:

```sh
agent-gh issue comment 123 --body-file checkpoint.md
agent-gh pr create --head prepared-branch --body-file pr.md
```

`agent-gh` reuses a cached token until five minutes before the token expires, then requests a new
one. It runs `gh` with the token in `GH_TOKEN` and with `GH_HOST=github.com`, and removes
`GITHUB_TOKEN`, `GH_ENTERPRISE_TOKEN`, and `GITHUB_ENTERPRISE_TOKEN` from `gh`'s environment. The
user's stored `gh` login is not used or modified.

| Command                    | Behavior                                                        |
| -------------------------- | --------------------------------------------------------------- |
| `agent-gh self --help`     | Print usage                                                     |
| `agent-gh self --version`  | Print the version                                               |
| `agent-gh self status`     | Print the configuration, cache path, and cached token expiry    |
| `agent-gh self refresh`    | Request a new token, for example after the App's access changed |
| `agent-gh self hook-check` | Check a Claude Code or Codex `PreToolUse` payload on stdin      |

`agent-gh` exits with `gh`'s exit status; on Windows, a status outside 0–255 becomes 1. When
`agent-gh` fails before starting `gh`, it prints `agent-gh: <message>` to stderr and exits 1, or
exits 2 for a wrapper usage error.

## Hook

Add the hook to Claude Code's `.claude/settings.json`, or to Codex's `.codex/hooks.json`:

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [{ "type": "command", "command": "agent-gh self hook-check" }]
      }
    ]
  }
}
```

When a Bash command runs `gh`, `gh.exe`, or a path ending in either, the hook exits 2 with a stderr
message that tells the agent to use `agent-gh`. For other commands, the hook exits 0 without
output, which leaves the harness's normal approval flow in place. The hook also exits 2 on invalid
JSON, a missing `tool_input.command`, a payload over 1 MiB, or a command that nests substitutions
more than 64 levels deep.

The hook parses pipelines, `&&`, `||`, `;`, subshells, command substitution, quoting, leading
variable assignments, and redirections. It skips heredoc bodies and does not detect `gh` invoked
through a variable, an alias, `bash -c`, `eval`, a script, or a wrapper command such as `env`,
`sudo`, or `xargs`. For a command it cannot parse, such as one with an unclosed quote, the hook
exits 0 without output.

## Limitations

- The hook reduces accidental use of personal credentials; scripts and HTTP clients can still use
  any credentials the agent's user can read.
- On Unix, the cache directory is created with mode `0700` and the cache file with `0600`. On
  Windows, the cache inherits the ACL of `%LOCALAPPDATA%`, which can grant access to other accounts
  such as Codex sandbox users. A cached token expires within an hour.
- `gh` sends `GH_TOKEN` to github.com and `*.ghe.com` hosts, so when a `--repo` value or URL names
  a `*.ghe.com` host, that host receives the installation token.
- GitHub Enterprise Server, multiple Apps or installations, and Git push authentication are not
  supported. Git commands that the agent runs directly use the user's Git identity and credentials.
- Bot attribution has not yet been verified for creating and editing issues, commenting, or
  creating pull requests. Access to a user-owned Project and the hook's behavior in a live Claude
  Code or Codex session have not been verified either.

## Development

Install the toolchains and the dependency-check tools:

```sh
rustup toolchain install stable --profile minimal --component clippy
rustup toolchain install nightly --profile minimal --component rustfmt
cargo +stable install --locked cargo-audit cargo-machete
```

The `just` recipes require [`just`](https://github.com/casey/just) and a POSIX `sh`; on Windows, run
them from Git Bash. `just check-msrv` also requires `bash` and `jq`.

```sh
just --list          # list recipes
just check           # formatting, Clippy, tests, docs, and dependency checks
just fix             # apply Clippy fixes and formatting, then lint
just check-msrv agent-gh
just install-hooks   # use the committed .githooks/ directory in this clone
```

The integration tests in `agent-gh/tests/cli/` run the built executable against a fake `gh` that
`cargo test` builds from `agent-gh/examples/fake_gh.rs`. No test contacts GitHub.

The repository's `.claude/settings.json` and `.codex/hooks.json` run `agent-gh self hook-check`
before each Bash command. Run `just install-hooks` once per clone; the `.githooks/commit-msg` hook
then adds a `Co-authored-by: kvnxiao-agent[bot]` trailer to commits made from Claude Code or Codex
sessions and leaves other commits unchanged.

Formatting uses nightly rustfmt because `rustfmt.toml` enables unstable options. Configure the
editor to format with nightly as well; for rust-analyzer:

```json
{
  "rust-analyzer.rustfmt.overrideCommand": ["rustup", "run", "nightly", "rustfmt"]
}
```

## License

[MIT](LICENSE)

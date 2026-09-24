# agent-gh

`agent-gh` runs the official GitHub CLI (`gh`) with a GitHub App installation token, so issues,
comments, and pull requests that an agent creates are attributed to the App's bot account instead
of the user. Each repository selects a profile, which names the App and one of its installations.
Commands that match the profile's `run_as_user` list run with the user's credentials; see
[Design](#design). A hook in the agent harness (Claude Code or Codex) blocks direct `gh` commands
and tells the agent to use `agent-gh`. A Git `commit-msg` hook credits the App's bot as a co-author
of agent commits; see [Co-author trailer](#co-author-trailer).

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
   [profiles.personal]
   app_id = 1234567
   installation_id = 98765432
   private_key_path = "personal.private-key.pem"
   run_as_user = ["pr create", "pr new"]

   [profiles.work]
   app_id = 7654321
   installation_id = 12345678
   private_key_path = "work.private-key.pem"
   ```

   Each `[profiles.<name>]` table is complete: a profile does not inherit fields from another
   profile. A profile name contains only ASCII letters, digits, `_`, and `-`. A relative
   `private_key_path` resolves against the configuration file's directory, and `run_as_user`
   defaults to an empty list. The file must define at least one profile, and `agent-gh` rejects
   unknown keys, including keys outside a `[profiles.<name>]` table.

4. In each repository the agent works in, select a profile and install the commit hook:

   ```sh
   agent-gh self setup-git-hooks personal
   ```

   The command requires Git 2.54 or later. It obtains the profile's installation token and the
   App's bot identity, and requests each one from GitHub unless it is already cached; the command
   fails when GitHub rejects the App ID, the private key, or the installation ID. After obtaining
   both, the command writes the selection and the hook to the repository's `.git/config`. Running
   it again with another profile switches the repository to that profile.

The configuration file is `~/.config/agent-gh/config.toml` on every platform, including Windows,
so one dotfiles layout covers all of them. When `XDG_CONFIG_HOME` is set to an absolute path, it
replaces `~/.config`. The caches stay in the platform cache directory:

| Platform | Cache directory              |
| -------- | ---------------------------- |
| Linux    | `~/.cache/agent-gh/`         |
| macOS    | `~/Library/Caches/agent-gh/` |
| Windows  | `%LOCALAPPDATA%\agent-gh\`   |

On Linux, `XDG_CACHE_HOME` replaces `~/.cache`. `AGENT_GH_CONFIG` names a different configuration
file, and `AGENT_GH_CACHE_DIR` names a different cache directory. The cache directory contains one
`token-<app_id>-<installation_id>.toml` file per installation and one `identity-<app_id>.toml` file
per App; an identity file contains the App's slug and the bot's user ID.

## Design

`agent-gh` reads the profile name from the `agent-gh.profile` key in the Git config of the
repository in the working directory, with `git config --local`. That file is local to each
repository on disk: `git clone` never copies it, and linked worktrees share it. Global and system
Git config cannot select a profile, and there is no default profile. When the working directory is
outside a repository, or the repository does not select a profile, a proxied command and
`agent-gh self refresh` print a message that names `agent-gh self setup-git-hooks <profile>` and
exit 1 without contacting GitHub or running `gh`. When the repository selects a profile that the
configuration file does not define, they exit 1 with a message that lists the defined profiles.
`-R other/repo` and `GH_REPO` do not change the profile: `gh` targets the other repository with the
working repository's profile.

`agent-gh` runs every `gh` command with the profile's installation token, except commands that
match an entry of the profile's optional `run_as_user` list:

```toml
run_as_user = ["pr create", "pr new"]
```

A command matches an entry when its leading arguments equal the entry's whitespace-separated words,
compared case-sensitively. The entry `pr create` matches `agent-gh pr create --fill`, but not
`agent-gh pr --repo owner/repo create`, which runs with the installation token. For a matching
command, `agent-gh` does not request a token and runs `gh` with the environment unchanged, so `gh`
uses the user's credentials: `GH_TOKEN` or `GITHUB_TOKEN` when set, and otherwise the login stored
by `gh auth login`.

`agent-gh` defaults to the installation token to keep every unmatched command within the App's
access. The installation's repository selection and permissions limit the installation token, while
the user's credentials usually reach every repository the user can access. If the user's
credentials were the default, every command missing from the list would run with them without a
warning, including `gh api`, `gh` aliases, and subcommands that later `gh` releases add. With the
installation token as the default, a command missing from the list is attributed to the bot, so the
list needs to name only the commands whose attribution matters.

GitHub makes the author of a pull request the author of its squash-merged commit. When
`run_as_user` lists `pr create` and its alias `pr new`, `gh` opens the agent's pull requests with
the user's credentials, so the user authors the squash commits of those pull requests. GitHub's
default squash message includes the `Co-authored-by` trailers of the pull request's commits, so the
squash commit also credits the bot when the commit hook added the bot's trailer to those commits.

## Usage

`agent-gh` accepts the same arguments as `gh`:

```sh
agent-gh issue comment 123 --body-file checkpoint.md
agent-gh pr create --head prepared-branch --body-file pr.md
```

For a command that does not match `run_as_user`, `agent-gh` reuses a cached token until five
minutes before the token expires, then requests a new one. It runs `gh` with the token in
`GH_TOKEN` and with `GH_HOST=github.com`, and removes `GITHUB_TOKEN`, `GH_ENTERPRISE_TOKEN`, and
`GITHUB_ENTERPRISE_TOKEN` from `gh`'s environment, so `gh` sends the installation token instead of
the user's stored github.com login. `agent-gh` never modifies the stored login.

| Command                                   | Behavior                                                                          |
| ----------------------------------------- | --------------------------------------------------------------------------------- |
| `agent-gh self --help`                    | Print usage                                                                       |
| `agent-gh self --version`                 | Print the version                                                                 |
| `agent-gh self status`                    | Print the configuration, the repository's profile, the caches, and the hook state |
| `agent-gh self refresh`                   | Request a new token and App identity, for example after the App changed           |
| `agent-gh self setup-git-hooks <profile>` | Select a profile for the repository and install the commit hook                   |
| `agent-gh self remove-git-hooks`          | Remove the profile selection and the commit hook from the repository              |
| `agent-gh self co-author`                 | Print the co-author value for an agent commit; the commit hook runs it            |
| `agent-gh self hook-check`                | Check a Claude Code or Codex `PreToolUse` payload on stdin                        |

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

## Co-author trailer

`agent-gh self setup-git-hooks <profile>` writes these keys to the repository's `.git/config`:

```ini
[agent-gh]
	profile = personal
[hook "agent-gh.commit-msg"]
	event = commit-msg
	command = git interpret-trailers --in-place --trim-empty --if-exists addIfDifferent --trailer \"Co-authored-by: $(agent-gh self co-author)\"
```

Git 2.54 introduced config-defined hooks, the `hook.<name>.command` and `hook.<name>.event` keys;
older Git ignores them. For each commit, Git runs the hook command through `sh` and passes the
path of the message file as an argument. `sh` looks up `agent-gh` on `PATH` and runs
`agent-gh self co-author`, and `git interpret-trailers` adds a `Co-authored-by` trailer with the
output of `agent-gh self co-author` as the value. For an agent commit, the command prints the bot's
name and email:

```text
Co-authored-by: kvnxiao-agent[bot] <332833177+kvnxiao-agent[bot]@users.noreply.github.com>
```

GitHub links the trailer to the bot account by the email address, which contains the bot's user
ID. `agent-gh` requests the App's slug from `GET /app` with the App JWT and the bot user ID from
`GET /users/<slug>[bot]` with the installation token, and caches both in `identity-<app_id>.toml`.
A cached identity is reused without network requests until `agent-gh self refresh` fetches it
again, for example after the App is renamed.

A commit is an agent commit when one of these variables is set to a non-empty value:

| Agent              | Variable                    |
| ------------------ | --------------------------- |
| Claude Code        | `CLAUDE_CODE_CHILD_SESSION` |
| Codex              | `CODEX_THREAD_ID`           |
| Gemini CLI         | `GEMINI_CLI`                |
| GitHub Copilot CLI | `COPILOT_CLI`               |
| Cursor             | `CURSOR_AGENT`              |

Claude Code's IDE extensions also set `CLAUDECODE` in the user's own terminals, so `CLAUDECODE`
does not mark an agent commit. For other commits, `agent-gh self co-author` exits 0 without output
and without reading any file, and `--trim-empty` removes the empty trailer. A local repository
where the user never ran the setup command has none of the keys that setup writes, so a
contributor's commits never credit another user's bot.

- With `--if-exists addIfDifferent`, amending a commit that has the trailer does not add a second
  copy. Other co-author trailers stay in the message unchanged, including the one Claude Code adds
  and those passed with `git commit --trailer`.
- Commits that skip the `commit-msg` hook do not get the trailer: `git commit --no-verify`,
  cherry-pick, revert, and the `pick` steps of `git rebase`. To add the trailer by hand, run
  `git commit --no-verify --trailer "Co-authored-by: $(agent-gh self co-author)"` from an agent's
  shell.
- `git interpret-trailers` rewrites the trailer block of every commit in the repository, including
  the user's own commits. `--trim-empty` removes empty trailers that the author wrote, such as a
  bare `Fixes:` line, and each remaining trailer is written with one space after the colon.
- Config-defined hooks run before the hook scripts in `.git/hooks` or `core.hooksPath`, and setup
  does not change `core.hooksPath` or write files under `.git/hooks`. husky, which sets
  `core.hooksPath=.husky/_`, and pre-commit, which installs scripts in `.git/hooks`, keep running
  their hooks. pre-commit refuses to install while `core.hooksPath` is set, as in a husky
  repository.

The hook never blocks a commit. When `agent-gh self co-author` cannot load the configuration,
resolve the profile, or obtain the identity, it prints
`agent-gh: <cause>; committed without the co-author trailer` to stderr and exits 0 without printing
to stdout. On an identity cache miss, the command makes up to three requests (the App, an
installation token when the token cache also misses, and the bot user), each limited to 30 seconds.
When `agent-gh` is missing from `PATH`, `sh` prints a `command not found` error, and Git commits
without the trailer.

`agent-gh self remove-git-hooks` removes every key that setup writes, including the profile
selection, so later proxied commands and `agent-gh self refresh` in the repository exit with the
setup message.

## Limitations

- The `PreToolUse` hook reduces accidental use of personal credentials, and commands that match
  `run_as_user` use them by design. Scripts, HTTP clients, and the Git commands that `gh` runs can
  still use any credentials the agent's user can read.
- Every proxied command requires a working directory inside a repository that selects a profile,
  including commands such as `gh auth status` that do not use a repository.
- On Unix, the cache directory is created with mode `0700` and each cache file with `0600`. On
  Windows, the cache inherits the ACL of `%LOCALAPPDATA%`, which can grant access to other accounts
  such as Codex sandbox users. A cached token expires within an hour.
- `gh` sends `GH_TOKEN` to github.com and `*.ghe.com` hosts, so when a `--repo` value or URL names
  a `*.ghe.com` host, that host receives the installation token.
- GitHub Enterprise Server and Git push authentication are not supported. Git commands that the
  agent runs directly use the user's Git identity and credentials.
- Bot attribution has not yet been verified for creating and editing issues or for commenting.
  Access to a user-owned Project and the `PreToolUse` hook's behavior in a live Claude Code or
  Codex session have not been verified either.

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
just install         # install agent-gh from this checkout with cargo install --locked
```

The integration tests in `agent-gh/tests/cli/` run the built executable against a fake `gh` that
`cargo test` builds from `agent-gh/examples/fake_gh.rs`. No test contacts GitHub. The integration
tests run `git` from `PATH`, and the tests that install the commit hook require Git 2.54 or later.

The repository's `.claude/settings.json` and `.codex/hooks.json` run `agent-gh self hook-check`
before each Bash command. Run `agent-gh self setup-git-hooks <profile>` once in each local
repository to select the profile that `agent-gh` uses in that repository and to credit the App's bot
on agent commits. If the repository's `core.hooksPath` is `.githooks`, also run
`git config --unset core.hooksPath`.

Formatting uses nightly rustfmt because `rustfmt.toml` enables unstable options. Configure the
editor to format with nightly as well; for rust-analyzer:

```json
{
  "rust-analyzer.rustfmt.overrideCommand": ["rustup", "run", "nightly", "rustfmt"]
}
```

## License

[MIT](LICENSE)

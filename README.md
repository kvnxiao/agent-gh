# agent-gh

`agent-gh` runs the official GitHub CLI (`gh`) with the user's normal `gh` authentication. Commands
that match the selected profile's `run_as_bot` rules run with a GitHub App installation token
instead, so they act as the App's bot account. A typical profile sends issue comments, pull
request comments, and comment-only reviews to the bot and leaves every other command to the user.
Each repository selects a profile, which names the App, one of its installations, and the rules;
see [Design](#design). A hook in the agent harness (Claude Code or Codex) blocks direct `gh`
commands and tells the agent to use `agent-gh`. An optional Git `commit-msg` hook credits the App's
bot as a co-author of agent commits; see [Co-author trailer](#co-author-trailer).

## Setup

1. Create a GitHub App with the permissions that the bot commands need, such as Issues (read and
   write) and Pull requests (read and write). Install it on the repositories the agent works in;
   the installation's repository selection and permissions limit every installation token.
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
   run_as_bot = [
     "issue comment",
     "pr comment",
     "pr review --comment",
     "pr review -c",
     "api repos/*/comments",
     "api repos/*/comments/*",
   ]

   [profiles.work]
   app_id = 7654321
   installation_id = 12345678
   private_key_path = "work.private-key.pem"
   ```

   Each `[profiles.<name>]` table is complete: a profile does not inherit fields from another
   profile. A profile name contains only ASCII letters, digits, `_`, and `-`. A relative
   `private_key_path` resolves against the configuration file's directory. `run_as_bot` is
   optional; [Routing rules](#routing-rules) defines its entries. The file must define at least one
   profile, and `agent-gh` rejects unknown keys, including keys outside a `[profiles.<name>]` table.

4. In each repository the agent works in, select a profile:

   ```sh
   agent-gh self setup personal
   ```

   The command loads the configuration file, checks that it defines the profile, and writes the
   selection to the repository's `.git/config`. It makes no GitHub request, installs no hook, keeps
   hooks that are already installed, and does not require Git 2.54. Running it again with another
   profile switches the repository to that profile.

5. Optionally, install the commit hook in the same step or later:

   ```sh
   agent-gh self setup personal --git-hooks
   agent-gh self install-git-hooks
   ```

   `--git-hooks` selects the profile and installs the hook; `install-git-hooks` installs the hook
   for the profile the repository already selects. Both require Git 2.54 or later. Both obtain the
   profile's installation token and the App's bot identity, and request each one from GitHub unless
   it is already cached; they fail when GitHub rejects the App ID, the private key, or the
   installation ID. They write to `.git/config` only after obtaining both.
   If `setup <profile> --git-hooks` fails to write the hook after it writes the profile, the new
   profile stays selected without a hook, and running `agent-gh self install-git-hooks` completes
   the installation.

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

### Profile selection

`agent-gh` reads the profile name from the `agent-gh.profile` key in the Git config of the
repository in the working directory, with `git config --local`. That file is local to each
repository on disk: `git clone` never copies it, and linked worktrees share it. Global and system
Git config cannot select a profile, and there is no default profile. When the working directory is
outside a repository, or the repository does not select a profile, a proxied command and
`agent-gh self refresh` print a message that names `agent-gh self setup <profile>` and exit 1
without contacting GitHub or running `gh`. When the repository selects a profile that the
configuration file does not define, they exit 1 with a message that lists the defined profiles.
`-R other/repo` and `GH_REPO` do not change the profile: `gh` targets the other repository with the
working repository's profile.

### Authentication

`agent-gh` has one default identity: the user's normal `gh` authentication. A command that matches
no `run_as_bot` rule runs `gh` with the environment unchanged, so `gh` resolves credentials as
usual: `GH_TOKEN` or `GITHUB_TOKEN` when set, and otherwise the login stored by `gh auth login`.
Because environment tokens take part, normal authentication does not guarantee a particular named
account. For these commands, `agent-gh` reads neither the App's private key nor the token cache.

A command that matches the rules runs with the installation token. `agent-gh` reuses a cached token
until five minutes before the token expires, then requests a new one. It runs `gh` with the token
in `GH_TOKEN` and with `GH_HOST=github.com`, and removes `GITHUB_TOKEN`, `GH_ENTERPRISE_TOKEN`, and
`GITHUB_ENTERPRISE_TOKEN` from `gh`'s environment, so `gh` sends the installation token instead of
the user's credentials. `agent-gh` never modifies the stored login.

Both paths share these guarantees:

- `gh` receives the original arguments, stdin, working directory, and output streams, and
  `agent-gh` passes on `gh`'s exit status as [Usage](#usage) describes.
- `agent-gh` selects credentials once per command. It never retries a failed command under the
  other identity.
- A profile's rules apply in every repository that selects the profile.

The rules select attribution; they do not restrict access. Unmatched commands have the user's
access, which usually reaches every repository the user can access, while matched commands have
only the installation's repository selection and permissions.

### Routing rules

`run_as_bot` is an optional list of strings in a `[profiles.<name>]` table. When the list is
omitted or empty, every command runs with normal authentication.

- **Entries:** an entry is a sequence of tokens separated by whitespace. The leading tokens that do
  not start with `-` are positional words, and the remaining tokens start with `-` and are flag
  tokens. Leading whitespace is ignored, so an entry whose first non-whitespace character is `!` is
  an exception, and the rest of it is parsed the same way.
- **Matching an entry:** a command matches an entry when each positional word matches the command
  argument at the same position, counting from the first argument after `agent-gh`, and each flag
  token matches at least one argument anywhere in the command.
- **Matching a token:** a token matches only a whole argument. `*` matches any sequence of
  characters, including an empty one and `/`. Every other character matches only itself,
  case-sensitively; no other glob or regular-expression syntax exists. An argument that is not
  valid UTF-8 matches no token.
- **Result:** a command runs as the bot when it matches at least one entry and no exception. Entry
  order does not matter.

Loading the configuration fails, with an error that names the profile, for these entries:

- an empty or whitespace-only entry
- an entry without a positional word, including a bare `!`
- an entry with a positional word after a flag token

An exception removes commands from the set that broader entries match. With `run_as_bot = ["pr *", "!pr merge"]`, every
`pr` subcommand except `pr merge` runs as the bot, and `pr` alone runs as the user because `*` needs
an argument to match.

### Comments configuration

This configuration sends comments to the bot:

```toml
run_as_bot = [
  "issue comment",
  "pr comment",
  "pr review --comment",
  "pr review -c",
  "api repos/*/comments",
  "api repos/*/comments/*",
]
```

It routes these commands to the bot:

- `issue comment` and `pr comment`, including `--edit-last` and `--delete-last`, which act on the
  bot's own comments.
- `pr review` with `--comment` or `-c`.
- `api` requests whose argument after `api` is an endpoint that matches one of these patterns:
  - `repos/*/comments`: `repos/`, then any characters, then `/comments`, such as
    `repos/o/r/issues/1/comments`, `repos/o/r/pulls/1/comments`, and
    `repos/o/r/commits/<sha>/comments`. `repos/comments` does not match.
  - `repos/*/comments/*`: `repos/`, then any characters, then `/comments/`, then any characters,
    such as `repos/o/r/pulls/1/comments/2/replies` and `repos/o/r/issues/comments/3`, which covers
    replies, edits, and deletes.

The matcher compares arguments without parsing `gh`'s syntax, which has these consequences:

- `pr review --approve` and `pr review --request-changes` run as the user, because they have no
  `--comment` or `-c`.
- Reads on matching paths run as the bot, for example `api repos/o/r/issues/1/comments` without
  fields.
- A flag value spelled exactly like a rule's flag token matches: `pr review 1 --approve --body -c`
  runs as the bot.
- These forms match no entry, so the command runs as the user:
  - A flag or other argument before a rule's positional words, as in
    `pr -R o/r review 1 --comment` or `api -X POST repos/o/r/issues/1/comments`.
  - Combined short flags, such as `-bc`.
  - An endpoint with a leading `/`, such as `/repos/o/r/issues/1/comments`.
- Aliases match literally: `pr new`, `issue new`, user aliases, and extensions each need their own
  entry.
- `--web` runs under whichever identity the rules select, but the browser step acts as the browser
  session, so `agent-gh` promises no bot attribution for browser actions.
- Rules do not inspect hosts, and a matched command always gets the bot environment above.

Other ways to create a comment, such as GraphQL mutations through `api graphql` or a review with
inline comments sent to `repos/o/r/pulls/1/reviews`, match none of these entries and run as the
user.

## Usage

`agent-gh` accepts the same arguments as `gh`. With the comments configuration, the first command
runs as the bot and the second as the user:

```sh
agent-gh issue comment 123 --body-file checkpoint.md
agent-gh pr create --head prepared-branch --body-file pr.md
```

| Command                                     | Behavior                                                                          |
| ------------------------------------------- | --------------------------------------------------------------------------------- |
| `agent-gh self --help`                      | Print usage                                                                       |
| `agent-gh self --version`                   | Print the version                                                                 |
| `agent-gh self status`                      | Print the configuration, the repository's profile, the caches, and the hook state |
| `agent-gh self refresh`                     | Request a new token and App identity, for example after the App changed           |
| `agent-gh self setup <profile>`             | Select a profile for the repository                                               |
| `agent-gh self setup <profile> --git-hooks` | Select a profile for the repository and install the commit hook                   |
| `agent-gh self install-git-hooks`           | Install the commit hook for the repository's selected profile                     |
| `agent-gh self remove-git-hooks`            | Remove the commit hook and keep the profile selection                             |
| `agent-gh self co-author`                   | Print the co-author value for an agent commit; the commit hook runs it            |
| `agent-gh self hook-check`                  | Check a Claude Code or Codex `PreToolUse` payload on stdin                        |

`self status` and `self refresh` work whether or not the commit hook is installed. In a repository
that selects `personal` without the hook, `agent-gh self status` prints:

```text
config: /home/me/.config/agent-gh/config.toml
profiles: personal, work
repository: /home/me/src/project
profile: personal
app_id: 1234567
installation_id: 98765432
private_key_path: /home/me/.config/agent-gh/personal.private-key.pem
run_as_bot: ["issue comment", "pr comment", "pr review --comment", "pr review -c", "api repos/*/comments", "api repos/*/comments/*"]
token cache: /home/me/.cache/agent-gh/token-1234567-98765432.toml
token: valid until 2026-09-27T21:00:00Z
identity cache: /home/me/.cache/agent-gh/identity-1234567.toml
co-author: kvnxiao-agent[bot] <332833177+kvnxiao-agent[bot]@users.noreply.github.com>
commit hook: not installed
```

Before the caches are filled, `token` reads `none cached` and `co-author` reads `not cached`.

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

The hook works independently of repository setup: `agent-gh self hook-check` reads neither the
configuration file nor the repository's profile. When a Bash command runs `gh`, `gh.exe`, or a path
ending in either, the hook exits 2 and prints this message to stderr, with the matched word in
place of `gh`:

```text
agent-gh: `gh` runs the GitHub CLI without agent-gh. Replace `gh` with `agent-gh`, which runs the command with the user's normal gh authentication, or as the GitHub App bot when the command matches the profile's run_as_bot rules.
```

For other commands, the hook exits 0 without output, which leaves the harness's normal approval
flow in place. The hook also exits 2 on invalid JSON, a missing `tool_input.command`, a payload
over 1 MiB, or a command that nests substitutions more than 64 levels deep.

The hook parses pipelines, `&&`, `||`, `;`, subshells, command substitution, quoting, leading
variable assignments, and redirections. It skips heredoc bodies and does not detect `gh` invoked
through a variable, an alias, `bash -c`, `eval`, a script, or a wrapper command such as `env`,
`sudo`, or `xargs`. For a command it cannot parse, such as one with an unclosed quote, the hook
exits 0 without output.

## Co-author trailer

`agent-gh self setup <profile>` writes the `agent-gh.profile` key, and hook installation writes the
`hook.agent-gh.commit-msg` keys, to the repository's `.git/config`:

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
where the user never installed the hook has no hook keys, so a contributor's commits never credit
another user's bot.

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
- Config-defined hooks run before the hook scripts in `.git/hooks` or `core.hooksPath`, and hook
  installation does not change `core.hooksPath` or write files under `.git/hooks`. husky, which
  sets `core.hooksPath=.husky/_`, and pre-commit, which installs scripts in `.git/hooks`, keep
  running their hooks. pre-commit refuses to install while `core.hooksPath` is set, as in a husky
  repository.

GitHub makes the author of a pull request the author of its squash-merged commit. With the comments
configuration, `pr create` runs with the user's credentials, so the user authors the squash commits
of the agent's pull requests. GitHub's default squash message includes the `Co-authored-by`
trailers of the pull request's commits, so the squash commit also credits the bot when the commit
hook added the bot's trailer to those commits.

The hook never blocks a commit. When `agent-gh self co-author` cannot load the configuration,
resolve the profile, or obtain the identity, it prints
`agent-gh: <cause>; committed without the co-author trailer` to stderr and exits 0 without printing
to stdout. On an identity cache miss, the command makes up to three requests (the App, an
installation token when the token cache also misses, and the bot user), each limited to 30 seconds.
When `agent-gh` is missing from `PATH`, `sh` prints a `command not found` error, and Git commits
without the trailer.

`agent-gh self remove-git-hooks` removes the `hook.agent-gh.commit-msg` keys and keeps the profile
selection, so proxied commands and `agent-gh self refresh` keep working in the repository. Running
it in a repository without the hook succeeds and changes nothing. To clear the profile selection,
run `git config --local --unset agent-gh.profile`.

## Limitations

- The `PreToolUse` hook reduces accidental direct use of `gh`; it does not keep the agent away from
  the user's credentials, which unmatched commands use by design. Scripts, HTTP clients, and the
  Git commands that `gh` runs can also use any credentials the agent's user can read.
- Every proxied command requires a working directory inside a repository that selects a profile,
  including commands such as `gh auth status` that do not use a repository.
- On Unix, the cache directory is created with mode `0700` and each cache file with `0600`. On
  Windows, the cache inherits the ACL of `%LOCALAPPDATA%`, which can grant access to other accounts
  such as Codex sandbox users. A cached token expires within an hour.
- `gh` sends `GH_TOKEN` to github.com and `*.ghe.com` hosts, so when a matched command's `--repo`
  value or URL names a `*.ghe.com` host, that host receives the installation token.
- GitHub Enterprise Server and Git push authentication are not supported. Git commands that the
  agent runs directly use the user's Git identity and credentials.
- Bot attribution has not been verified against GitHub for creating and editing issues or for
  commenting. Access to a user-owned Project and the `PreToolUse` hook's behavior in a live Claude
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
just install         # install agent-gh from this checkout with cargo install --locked
```

The integration tests in `agent-gh/tests/cli/` run the built executable against a fake `gh` that
`cargo test` builds from `agent-gh/examples/fake_gh.rs`. No test contacts GitHub. The integration
tests run `git` from `PATH`, and the tests that install the commit hook require Git 2.54 or later.

The repository's `.claude/settings.json` and `.codex/hooks.json` run `agent-gh self hook-check`
before each Bash command. Run `agent-gh self setup <profile>` once in each local repository to
select the profile that `agent-gh` uses in that repository, and add `--git-hooks` to credit the
App's bot on agent commits. If the repository's `core.hooksPath` is `.githooks`, also run
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

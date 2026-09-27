# AGENTS.md

Instructions for coding agents working in this repository.

## Layout

The root `Cargo.toml` defines a virtual Cargo workspace. The `agent-gh/` member builds the
`agent-gh` executable. The root manifest also contains the lint baseline, the shared package
metadata, and the build profiles.

## Commands

Run repository tasks through `just`. The recipes run under POSIX `sh`; on Windows, run them from Git
Bash.

| Recipe                     | Action                                                        |
| -------------------------- | ------------------------------------------------------------- |
| `just fmt`                 | Format with nightly rustfmt.                                  |
| `just lint`                | Check formatting and run stable Clippy with `-D warnings`.    |
| `just fix`                 | Apply Clippy fixes and formatting, then run `just lint`.      |
| `just test`                | Run every test in the workspace.                              |
| `just doc`                 | Build documentation with rustdoc warnings denied.             |
| `just dependencies`        | Run `cargo audit` and `cargo machete`.                        |
| `just check`               | Run `lint`, `test`, `doc`, and `dependencies`.                |
| `just check-msrv agent-gh` | Check the package with its declared `rust-version` toolchain. |
| `just install`             | Install the `agent-gh` executable from this checkout.         |

Run `just check` before committing. Run a single test or the executable with Cargo directly:

```sh
cargo +stable test -p agent-gh --locked <test-name-filter>
cargo +stable run -p agent-gh --locked -- <args>
```

The integration tests in `agent-gh/tests/cli/` run the executable against a fake `gh` that
`cargo test` builds from `agent-gh/examples/fake_gh.rs`; the hook-failure tests also use a fake
`git` from `agent-gh/examples/fake_git.rs`. `cargo test --test cli` alone does not build the fakes;
run `cargo +stable build -p agent-gh --example fake_gh --example fake_git --locked` first. The tests
that install the commit hook require Git 2.54 or later on `PATH`.

## Toolchains

- Compile, test, and run Clippy on floating stable; format with floating nightly. Example:
  `cargo +nightly fmt --all`. Stable rustfmt skips the unstable options in `rustfmt.toml` with a
  warning and exits 0, so it can write formatting that `just lint` rejects.
- Pass `--locked` to Cargo commands; `Cargo.lock` is committed. Change dependencies with
  `cargo +stable add` or `cargo +stable remove`; after editing a manifest by hand, sync the lockfile
  with `cargo +stable update --workspace`.
- Declare `rust-version` only in `[workspace.package]`. `just check-msrv` reads the value from Cargo
  metadata.

## Rules

- Run every GitHub CLI command as `agent-gh` with the usual `gh` arguments, for example
  `agent-gh pr view 1`. `agent-gh` runs commands with the user's normal `gh` authentication and
  runs commands that match the profile's `run_as_bot` rules as the GitHub App's bot account. The
  App and the rules are set in the profile that `agent-gh self setup <profile>` selected for this
  repository.
- Before writing or reviewing `*.rs` files or Cargo, Clippy, rustfmt, or toolchain manifests, read
  `.agents/skills/rust-rules/SKILL.md` and the references it lists for the task.
- Before writing or reviewing `.github/workflows/**`, read
  `.agents/skills/github-actions-rules/SKILL.md`.
- Apply every skill edit to both `.agents/skills/` and `.claude/skills/`. When the copies match,
  `diff -r .agents/skills .claude/skills` prints nothing.
- Ask the user before adding a lint exception outside the rust-rules exception table or weakening
  the workspace lint baseline.
- Give each new member `[lints] workspace = true`, inherit the `[workspace.package]` fields, and
  place it in a root-level directory listed in `[workspace] members`.
- Before removing `publish = false` from a member, add `description`, `readme`, `keywords`, and
  `categories`; Clippy's `cargo_common_metadata` lint requires them for publishable packages.

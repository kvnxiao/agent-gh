# AGENTS.md

Instructions for coding agents working in this repository.

## Layout

The root `Cargo.toml` defines a virtual Cargo workspace. The `agent-gh/` member builds the
`agent-gh` executable. The root manifest also contains the lint baseline, the shared package
metadata, and the build profiles.

## Commands

Run repository tasks through `just`. The recipes run under POSIX `sh`; on Windows, run them from Git
Bash.

| Recipe                     | Action                                                          |
| -------------------------- | --------------------------------------------------------------- |
| `just fmt`                 | Format with nightly rustfmt.                                    |
| `just lint`                | Check formatting and run stable Clippy with `-D warnings`.      |
| `just fix`                 | Apply Clippy fixes and formatting, then run `just lint`.        |
| `just test`                | Run every test in the workspace.                                |
| `just doc`                 | Build documentation with rustdoc warnings denied.               |
| `just dependencies`        | Run `cargo audit` and `cargo machete`.                          |
| `just check`               | Run `lint`, `test`, `doc`, and `dependencies`.                  |
| `just check-msrv agent-gh` | Check the package with its declared `rust-version` toolchain.   |

Run `just check` before committing. Run a single test or the executable with Cargo directly:

```sh
cargo +stable test -p agent-gh --locked <test-name-filter>
cargo +stable run -p agent-gh --locked -- <args>
```

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

# agent-gh

`agent-gh` is a command-line tool written in Rust. The repository contains the Cargo workspace
skeleton; the executable does not implement any commands yet.

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
```

Formatting uses nightly rustfmt because `rustfmt.toml` enables unstable options. Configure the
editor to format with nightly as well; for rust-analyzer:

```json
{
  "rust-analyzer.rustfmt.overrideCommand": ["rustup", "run", "nightly", "rustfmt"]
}
```

## License

[MIT](LICENSE)

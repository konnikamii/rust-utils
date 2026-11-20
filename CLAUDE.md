# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Overview

A Cargo workspace of small, independent CLI tools, each a single binary crate under `crates/<name>/src/main.rs`:

- `env` — view/filter env vars, load `.env` files, temporarily set (`-s`) or persist (`-p`, `set`/`append`/`remove` subcommands) variables, and run a command with the modified environment (args after `--`).
- `tr` — `tree`-like directory viewer. Lib + bin: the parallel `rayon` scanner lives in `src/lib.rs` (public `tr::scan(path, &ScanOptions) -> Tree`, used by the Lumen desktop app), and `src/main.rs` only does CLI args, spinner, and `render_tree`. CLI-only deps (clap, colored) sit behind the default `cli` feature, which the bin requires; library users set `default-features = false`.
- `grep` — small recursive regex search (`walkdir` + `regex`), case-insensitive by default, reads stdin when no paths are given.

The binary name equals the crate directory name. Release packaging depends on this (it copies `target/release/<crate-dir>.exe`).

## Commands

```bash
cargo build                         # debug build of whole workspace
cargo build --workspace --release   # what CI runs
cargo run -p tr -- -sS -d 2 some/dir    # run a crate; args after `--`
cargo run -p env -- -F PATH
cargo run -p grep -- "pattern" src -n
cargo watch -w crates/tr -x "run -p tr"
cargo clippy --workspace
cargo test -p <crate> <test_name>   # only tr has tests (in lib.rs)
cargo build -p tr --lib --no-default-features   # check the lib builds without CLI deps
```

Use `cargo run -p <name> -- ...` (some older READMEs show the wrong `cargo run -- crates/<name> -- ...` form).

## Workspace conventions

- Version is shared: every crate uses `version.workspace = true`; the only version lives in `[workspace.package]` in the root `Cargo.toml`. Don't bump it by hand — the release workflow does.
- Dependencies are declared once in root `[workspace.dependencies]`, and crates reference them with `<dep>.workspace = true`.
- Adding a crate: `cargo init` in a new `crates/<name>` folder, then add it to `members` in the root `Cargo.toml`.
- Editions differ: `env` and `tr` use 2024, `grep` uses 2021.

## Platform notes

- The tools target Windows first. `env` and `tr` call `colored::control::set_virtual_terminal` (Windows-only) at startup, so they won't compile on non-Windows targets as written. This is why CI builds on `windows-latest`.
- `env` persistence is platform-specific: on Windows it uses `setx` to set and `reg delete` to remove; on Unix it edits export lines in the shell rc file.
- `tr` strips the `\\?\` prefix that `canonicalize()` adds on Windows paths.

## Release

`.github/workflows/release.yml` is manual only (`workflow_dispatch` with a patch/minor/major input). It bumps `[workspace.package].version` with sed, builds on Windows, zips the exes as `rust-utils-v<ver>-windows-x64.zip`, commits `chore(release): v<ver>`, tags `v<ver>`, pushes to the default branch, and creates a GitHub release. Pull after a release, because CI pushes a commit to `main`.

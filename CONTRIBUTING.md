# Contributing

This repository is a Cargo workspace containing small, independent CLI tools. Each tool is a binary crate in `crates/<name>/src/main.rs`, and the executable name is the crate directory name.

## Workspace layout

- `env` — view/filter environment variables, load `.env` files, set temporary or persistent variables, and run commands with a modified environment
- `tr` — parallel directory tree viewer using `rayon`
- `grep` — recursive regular-expression search using `walkdir` and `regex`

## Build and run

```powershell
cargo build
cargo build --workspace --release
cargo run -p tr -- -sS -d 2 some\dir
cargo run -p env -- -F PATH
cargo run -p grep -- "pattern" src -n
```

The binary name equals the crate directory name. Release packaging copies `target/release/<crate-name>.exe`.

## Tests and checks

No focused test suite is present yet:

```powershell
cargo test --workspace
cargo clippy --workspace
```

## Workspace conventions

- All crates use `version.workspace = true`; the shared version is `[workspace.package].version` in the root `Cargo.toml`.
- Dependencies are declared once in root `[workspace.dependencies]` and reused with `<dependency>.workspace = true`.
- `env` and `tr` use the Rust 2024 edition. `grep` uses the 2021 edition.
- The tools currently contain Windows-specific console APIs and build on `windows-latest` in CI.

## Adding a crate

```powershell
cargo init crates\<name>
```

Add the new crate to the workspace `members` array in the root `Cargo.toml`, then declare any dependencies in the workspace or crate manifest as appropriate.

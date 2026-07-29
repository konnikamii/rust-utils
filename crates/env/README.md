# env — environment variable manager

Small CLI for viewing and managing environment variables. Supports filtering, loading from `.env`-style files, temporary or persistent sets, and running commands with a modified environment.

## Usage

Runs as a small CLI. Examples:

```
cargo run -- crates/env --                  # show current environment
cargo run -- crates/env -- -F PATH          # show variables containing "PATH"
cargo run -- crates/env -- -f .env         # load variables from a .env file and show
cargo run -- crates/env -- -- MY_CMD arg    # run `MY_CMD` with the current/modified environment
```

Subcommands (persist/modify):

```
cargo run -- crates/env -- set NAME VALUE      # persistently set NAME=VALUE
cargo run -- crates/env -- append NAME VALUE   # append VALUE to NAME (persisted)
cargo run -- crates/env -- remove NAME         # remove persisted NAME
```

## Options (short)

- `-F`, `--filter`: Only show variables containing this substring (in key or value).
- `-f`, `--file`: Load variables from a `.env`-style file (NAME=VALUE lines).
- `-s`, `--set`: Temporarily set environment variable(s) in `NAME=VALUE` format (applies only while program runs).
- `-p`, `--persist`: Persistently set environment variable(s) in `NAME=VALUE` format (affects future processes).
- `--` trailing args: Command to run with the modified environment (pass command and its args after `--`).

## Behavior

- By default the tool lists all environment variables available to the process, sorted case-insensitively by key.
- If `--file` is provided, variables from the file are loaded into the process before display or command execution.
- `--set` applies variables only to the current run and any child process spawned; `--persist` and the `set`/`append` subcommands attempt to persist changes for future processes (platform-specific implementation).
- On Windows persistence uses `setx` or registry operations; on Unix-like systems persistence appends/removes export lines in the user's shell rc file.

## Examples

Show variables containing `HOME`:

```
cargo run -- crates/env -- -F HOME
```

Load a `.env` file and run a command with those variables:

```
cargo run -- crates/env -- -f .env -- my_command arg1
```

Persistently set a variable:

```
cargo run -- crates/env -- set MY_VAR "some value"
```

Append a value to `PATH` (persisted):

```
cargo run -- crates/env -- append PATH /custom/bin
```

## License / Notes

This is a small utility intended for local development and convenience. Persisting environment changes is platform-specific; review the implementation in `crates/env/src/main.rs` for exact behavior and safety considerations.

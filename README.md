# rust-utils

Small, focused command-line tools for Windows:

| Tool | What it does |
| --- | --- |
| [`env`](#env) | View, filter, load, set, and persist environment variables |
| [`tr`](#tr) | Display a directory as a colored tree with optional sizes and counts |
| [`grep`](#grep) | Search text, files, or directories with regular expressions |

All tools are standalone Windows executables. Rust is not required to use a downloaded release.

## Download

1. Open the [GitHub Releases page](https://github.com/konnikamii/rust-utils/releases).
2. Download `rust-utils-v<version>-windows-x64.zip` from the latest release.
3. Extract the ZIP.
4. Run the extracted `.exe` files from PowerShell or Command Prompt.

The archive contains `env.exe`, `tr.exe`, and `grep.exe`.

You can also run the tools from Cargo's source, as described in [CONTRIBUTING.md](CONTRIBUTING.md).

### Make a tool available everywhere

To use a tool from any directory, add the extracted folder to your user `PATH`. In PowerShell:

```powershell
$toolDirectory = "C:\Tools\rust-utils"
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
[Environment]::SetEnvironmentVariable("Path", "$userPath;$toolDirectory", "User")
```

Open a new terminal after changing `PATH`.

---

## env

`env` shows environment variables and can add temporary or persistent variables to the current process or a child command.

### View variables

```powershell
env
env --filter PATH
env -F "USER"
```

Variables are sorted alphabetically. `-F`/`--filter` matches text in either the variable name or value.

### Load a `.env` file

```dotenv
API_URL=https://example.com
DEBUG=true
```

```powershell
env --file .env
env -f .env -- node app.js
```

When running a command, `env` automatically loads `./.env` if that file exists. Use `--file` to choose a different file.

### Set variables temporarily

`-s`/`--set` changes variables only for the `env` process and the command that it starts:

```powershell
env -s API_URL=https://test.example -s DEBUG=true -- node app.js
```

Use `--` to separate the tool's options from the command and its arguments.

### Set or remove variables persistently

```powershell
env set API_URL "https://example.com"
env append PATH "C:\Tools\MyTool"
env remove API_URL
```

On Windows, these subcommands use `setx` or the user registry. Changes become visible to future processes; open a new terminal to see them.

The `--persist` option sets one or more variables persistently while also continuing a normal `env` invocation:

```powershell
env --persist API_URL=https://example.com
```

> Persistent environment changes affect your Windows user account. Review values carefully before setting or removing them.

### env options

| Option | Description |
| --- | --- |
| `-F`, `--filter <TEXT>` | Show variables whose names or values contain `TEXT` |
| `-f`, `--file <PATH>` | Load a `.env`-style file |
| `-s`, `--set <NAME=VALUE>` | Set a variable only for this run |
| `-p`, `--persist <NAME=VALUE>` | Persistently set a variable |
| `-- <COMMAND> [ARGS...]` | Run a command with the modified environment |

Run `env --help` for the built-in help.

---

## tr

`tr` prints a directory structure as a tree.

### Basic examples

```powershell
tr
tr C:\Projects
tr -d 3 C:\Projects
```

The directory defaults to the current directory. `-d`/`--depth` controls how many levels are shown; the default is `1`.

### Sizes and file counts

```powershell
tr -s -d 2 C:\Projects
tr -S C:\Projects
tr -n C:\Projects
tr -sS -n -d 3 C:\Projects
```

- `-s`/`--file-sizes` shows each file's size.
- `-S`/`--folder-sizes` shows cumulative directory sizes.
- `-n`/`--file-count` shows the number of immediate entries in directories.

### Filters and display

```powershell
tr -a
tr -F ".txt,.log"
tr -x "node_modules,target"
tr -o
tr -c
tr --no-sort
```

- `-a`/`--all` includes dot-prefixed hidden entries.
- `-F`/`--filter` keeps entries whose names contain one of the comma-separated text values.
- `-x`/`--hide-name` hides entries with exact comma-separated names.
- `-o`/`--folders-only` hides files.
- `-c`/`--clean` removes ANSI colors.
- `--no-sort` preserves filesystem order instead of sorting by name.

Run `tr --help` for the built-in help.

---

## grep

`grep` searches text using regular expressions.

### Search files and directories

```powershell
grep "TODO" src
grep "error|failed" C:\Logs -n
grep "fn main" src\main.rs -n
grep "TODO" src -F ".rs,.toml"
```

Search is case-insensitive by default. Use `-s`/`--case-sensitive` for case-sensitive matching.

Directories are searched recursively by default.

### Search piped text

```powershell
Get-ChildItem -Recurse -File | grep "name"
Get-Content app.log | grep "ERROR" -n
```

When input is piped to the tool and no paths are given, `grep` reads standard input.

### Options

| Option | Description |
| --- | --- |
| `-s`, `--case-sensitive` | Make matching case-sensitive |
| `-n`, `--line-number` | Include line numbers for file matches |
| `-r`, `--recursive` | Recurse into directories (enabled by default) |
| `-F`, `--filter <TEXT>` | Match file names containing comma-separated text values |

Run `grep --help` for the built-in help.

## Contributing

Build instructions, workspace conventions, and release packaging are in [CONTRIBUTING.md](CONTRIBUTING.md).

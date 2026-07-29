# tr — directory tree viewer

Small, fast directory tree printer inspired by the Unix `tree` utility.

## Usage

Runs as a small CLI. Example:

```
cargo run -- crates/tr -- [DIRECTORY]
```

If no directory is provided, the current directory is used.

## Options (short)

- `-a`, `--all`: Show hidden files (files starting with `.`).
- `-x`, `--hide-name`: Comma-separated exact names to hide (ignored when `-a` is set).
- `-d`, `--depth`: Maximum recursion depth (default: `1`).
- `-n`, `--file-count`: Show number of files in each directory.
- `-s`, `--file-sizes`: Show file sizes for files.
- `-S`, `--folder-sizes`: Show cumulative sizes for directories.
- `--no-sort`: Preserve filesystem order instead of sorting entries by name.
- `-F`, `--folders-only`: Show only folders (hide files).
- `-c`, `--clean`: Disable colored output.

## Behavior

- The tool scans the specified directory and prints a tree of entries.
- By default it hides dotfiles and symlinks and sorts entries by name.
- Size and file-count options may require reading child entries and therefore increase scan time.

## Example

Show a colored tree with file sizes and two levels of depth:

```
cargo run -- crates/tr -- -sS -d 2 path/to/dir
```

## License / Notes

This is a small utility intended for local development and inspection.

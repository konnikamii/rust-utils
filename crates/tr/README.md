# tr — directory tree viewer

Small, fast directory tree printer inspired by the Unix `tree` utility.

## Usage

Runs as a small CLI. Example:

```
cargo run -p tr -- [DIRECTORY]
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
- `-o`, `--folders-only`: Show only folders (hide files).
- `-F`, `--filter`: Comma-separated substrings; show only matching entries and the folders leading to them.
- `-c`, `--clean`: Disable colored output.

## Behavior

- The tool scans the specified directory and prints a tree of entries.
- By default it hides dotfiles and symlinks and sorts entries by name.
- Size and file-count options may require reading child entries and therefore increase scan time.

## Example

Show a colored tree with file sizes and two levels of depth:

```
cargo run -p tr -- -sS -d 2 path/to/dir
```

## Library

The scanner is also a library, so other Rust apps can call it directly instead of
spawning the CLI. Turn off default features to skip the CLI-only dependencies (clap, colored):

```toml
[dependencies]
tr = { path = "../rust-utils/crates/tr", default-features = false }
# or: tr = { git = "https://github.com/konnikamii/rust-utils", default-features = false }
```

```rust
let opts = tr::ScanOptions { depth: 2, dir_sizes: true, ..Default::default() };
let tree = tr::scan(std::path::Path::new("C:/some/dir"), &opts)?;
for entry in &tree.entries {
    println!("{} {} {:?}", entry.path.display(), entry.total_size, entry.modified);
}
```

`scan` returns only the entries the options select (the same ones the CLI prints), nested via
`children`. `Tree::total_size` and `child_count` also count entries that were hidden.
`scan` blocks and uses rayon's global thread pool, so in an async app (Tauri) run it inside
`spawn_blocking`.

For live progress, pass `progress: Some(Arc::new(tr::ScanProgress::default()))` and read
`current_dir()` / `dirs_read()` from another thread while the scan runs (the CLI spinner does this).

## License / Notes

This is a small utility intended for local development and inspection.

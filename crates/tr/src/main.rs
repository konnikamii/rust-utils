use clap::Parser;
use colored::{Colorize, control};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;

#[derive(Parser, Debug)]
struct Args {
    /// Directory to scan
    #[arg(default_value = ".")]
    directory: PathBuf,

    /// Show hidden files
    #[arg(short = 'a', long)]
    all: bool,

    /// Hide entries matching these exact names (comma-delimited). Ignored when -a is used
    #[arg(short = 'x', long, value_delimiter = ',')]
    hide_name: Vec<String>,

    /// Maximum depth
    #[arg(short = 'd', long, default_value_t = 1)]
    depth: usize,

    /// Show number of files in each directory
    #[arg(short = 'n', long)]
    file_count: bool,

    /// Show file sizes
    #[arg(short = 's', long)]
    file_sizes: bool,

    /// Show cumulative directory sizes
    #[arg(short = 'S', long)]
    folder_sizes: bool,

    /// Keep filesystem order instead of sorting entries by name
    #[arg(long)]
    no_sort: bool,

    /// Show only folders (hide files)
    #[arg(short = 'o', long)]
    folders_only: bool,

    /// Remove Colors from output
    #[arg(short = 'c', long)]
    clean: bool,

    /// Filter entries by comma-separated substrings in filename (e.g. ".txt,log")
    #[arg(short = 'F', long = "filter")]
    filter: Option<String>,
}

fn main() {
    control::set_virtual_terminal(true).unwrap();

    if let Err(e) = run() {
        eprintln!("tree: {e}");
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let args = Args::parse();
    let cwd = env::current_dir()?;

    let metadata = fs::symlink_metadata(&args.directory)?;

    if !metadata.file_type().is_dir() {
        println!("{} is a file", args.directory.display());
        return Err(io::Error::new(io::ErrorKind::Other, "Invalid directory"));
    }

    // Print the directory tree
    let start = cwd.join(&args.directory).canonicalize()?;
    let mut display_path = start.display().to_string();

    if let Some(stripped) = display_path.strip_prefix(r"\\?\") {
        display_path = stripped.to_string();
    }

    println!(
        "{}",
        if !args.clean {
            display_path.blue().bold()
        } else {
            display_path.normal()
        },
    );

    // Spinner while scanning/building the tree
    let spinner_running = Arc::new(AtomicBool::new(true));
    let spinner_flag = spinner_running.clone();
    let spinner_handle = thread::spawn(move || {
        let spinner = ['|', '/', '-', '\\'];
        let mut i: usize = 0;
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        while spinner_flag.load(Ordering::Relaxed) {
            let ch = spinner[i % spinner.len()];
            print!("\rScanning... {}", ch);
            let _ = handle.flush();
            i = i.wrapping_add(1);
            thread::sleep(std::time::Duration::from_millis(80));
        }
        // clear the spinner line
        print!("\r\x1b[2K");
        let _ = handle.flush();
    });

    let tree = build_tree(&start, 0, &args)?;

    spinner_running.store(false, Ordering::Relaxed);
    let _ = spinner_handle.join();

    render_tree(&tree, " ", &args);

    Ok(())
}

struct TreeEntry {
    name: String,
    should_print: bool,
    is_dir: bool,
    is_exec: bool,
    is_hidden: bool,
    is_symlink: bool,
    symlink_target: Option<String>,
    file_size: u64,
    total_size: u64,
    child_count: usize,
    children: Vec<TreeEntry>,
}

// Highlight occurrences of filter patterns within a name.
fn highlight_name_with_style<F>(name: &str, args: &Args, base_style: F) -> String
where
    F: Fn(&str) -> colored::ColoredString,
{
    if args.clean {
        return if name.is_empty() {
            String::new()
        } else {
            name.to_string()
        };
    }

    let filt = match &args.filter {
        Some(f) => f,
        None => return base_style(name).to_string(),
    };

    let patterns: Vec<&str> = filt
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();

    if patterns.is_empty() {
        return base_style(name).to_string();
    }

    let mut out = String::new();
    let mut remaining = name;

    while !remaining.is_empty() {
        // find earliest match among patterns
        let mut best: Option<(usize, &str)> = None;
        for &pat in &patterns {
            if let Some(idx) = remaining.find(pat) {
                match best {
                    Some((bidx, _)) if idx < bidx => best = Some((idx, pat)),
                    None => best = Some((idx, pat)),
                    _ => {}
                }
            }
        }

        if let Some((idx, pat)) = best {
            if idx > 0 {
                out.push_str(&base_style(&remaining[..idx]).to_string());
            }
            out.push_str(&remaining[idx..idx + pat.len()].red().to_string());
            remaining = &remaining[idx + pat.len()..];
        } else {
            out.push_str(&base_style(remaining).to_string());
            break;
        }
    }

    out
}

fn build_tree(path: &Path, depth: usize, args: &Args) -> io::Result<Vec<TreeEntry>> {
    if depth >= args.depth {
        return Ok(Vec::new());
    }

    let mut entries = read_visible_entries(path, args)?;

    if !args.no_sort {
        entries.sort_by_key(|(e, _)| e.file_name());
    }

    let mut tree = Vec::with_capacity(entries.len());
    for (entry, should_print) in entries {
        tree.push(build_entry(entry, should_print, depth, args)?);
    }

    Ok(tree)
}

fn build_entry(
    entry: fs::DirEntry,
    should_print: bool,
    depth: usize,
    args: &Args,
) -> io::Result<TreeEntry> {
    let path = entry.path();
    let name = entry.file_name().to_string_lossy().to_string();
    let metadata = entry.metadata()?;
    let is_dir = metadata.is_dir();
    let is_symlink = entry.file_type().map(|t| t.is_symlink()).unwrap_or(false);
    let is_hidden = name.starts_with('.');
    let is_exec = is_executable(&path, &metadata);
    let symlink_target = if is_symlink {
        Some(
            fs::read_link(&path)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| "unknown".to_string()),
        )
    } else {
        None
    };

    let file_size = if metadata.is_file() {
        metadata.len()
    } else {
        0
    };
    let mut total_size = file_size;
    let mut child_count = 0;
    let mut children = Vec::new();

    if is_dir {
        let should_read_children = depth + 1 < args.depth || args.file_count || args.folder_sizes;
        if should_read_children {
            let mut child_entries = read_visible_entries(&path, args)?; // Vec<(DirEntry, should_print)>

            child_count = child_entries.len();

            if !args.no_sort {
                child_entries.sort_by_key(|(e, _)| e.file_name());
            }

            let should_recurse = depth + 1 < args.depth || args.folder_sizes;
            if should_recurse {
                let should_store_children = depth + 1 < args.depth;
                for (child, child_should_print) in child_entries {
                    let child_node = build_entry(child, child_should_print, depth + 1, args)?;
                    total_size += child_node.total_size;
                    if should_store_children {
                        children.push(child_node);
                    }
                }
            }
        }
    }

    // Print directory if any children matches filters
    let final_should_print = if is_dir {
        should_print || children.iter().any(|c| c.should_print)
    } else {
        should_print
    };

    Ok(TreeEntry {
        name,
        should_print: final_should_print,
        is_dir,
        is_exec,
        is_hidden,
        is_symlink,
        symlink_target,
        file_size,
        total_size,
        child_count,
        children,
    })
}

fn read_visible_entries(path: &Path, args: &Args) -> io::Result<Vec<(fs::DirEntry, bool)>> {
    let mut entries = Vec::new();

    for entry in fs::read_dir(path)?.filter_map(Result::ok) {
        // When `--all` is set, it takes priority and we show everything.
        if args.all {
            entries.push((entry, true));
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let hidden_by_dot = name.starts_with('.');
        let hidden_by_name = args.hide_name.iter().any(|n| n == &name);
        let is_symlink = entry.file_type().map(|t| t.is_symlink()).unwrap_or(false);
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);

        // Determine should_print according to existing logic;
        // - no hidden files (by . or by name)
        // - no symlinks
        // - folders_only hides files
        let mut should_print =
            !hidden_by_dot && !hidden_by_name && !is_symlink && !(args.folders_only && !is_dir);

        // Apply substring filter
        if let Some(ref filt) = args.filter {
            let patterns: Vec<&str> = filt
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect();
            if !patterns.is_empty() {
                let name_matches = patterns.iter().any(|pat| name.contains(pat));
                should_print = should_print && name_matches;
            }
        }

        entries.push((entry, should_print));
    }

    Ok(entries)
}

fn render_tree(entries: &[TreeEntry], prefix: &str, args: &Args) {
    let last = entries.len().saturating_sub(1);

    for (i, entry) in entries.iter().enumerate() {
        // Visibility was computed during build; use the cached flag.

        let is_last = i == last;
        let connector = if is_last { "└── " } else { "├── " };

        let file_name = if entry.is_dir {
            // highlight matches inside directory name but keep directory color for non-matching parts
            if !args.clean {
                let h = highlight_name_with_style(&entry.name, args, |s| s.blue().bold());
                format!("{}/", h)
            } else {
                format!("{}/", entry.name)
            }
        } else if entry.is_exec {
            if !args.clean {
                let h = highlight_name_with_style(&entry.name, args, |s| s.green().bold());
                h
            } else {
                entry.name.clone()
            }
        } else if entry.is_hidden {
            if !args.clean {
                let h = highlight_name_with_style(&entry.name, args, |s| s.dimmed());
                h
            } else {
                entry.name.clone()
            }
        } else if entry.is_symlink {
            let target = entry
                .symlink_target
                .clone()
                .unwrap_or_else(|| "unknown".to_string());

            if !args.clean {
                let name_h = highlight_name_with_style(&entry.name, args, |s| s.cyan().bold());
                format!("{} -> {target}", name_h)
            } else {
                format!("{} -> {target}", entry.name)
            }
        } else {
            if !args.clean {
                highlight_name_with_style(&entry.name, args, |s| s.normal())
            } else {
                entry.name.clone()
            }
        };

        let size = if args.folder_sizes && entry.is_dir {
            format!(
                "{} ",
                format!("{:>8}", human_size(entry.total_size)).bright_green()
            )
        } else if args.file_sizes && !entry.is_dir {
            format!(
                "{} ",
                format!("{:>8}", human_size(entry.file_size)).bright_green()
            )
        } else {
            String::new()
        };

        let child_count = if args.file_count && entry.is_dir {
            format!(" ({})", entry.child_count)
        } else {
            String::new()
        };

        if entry.should_print {
            println!(
                "{}{}{}{}{}",
                prefix, connector, size, file_name, child_count
            );
        }

        // Always recurse into directories so sizes/counts include hidden entries.
        if entry.is_dir && !entry.children.is_empty() {
            let next_prefix = if is_last {
                format!("{prefix}    ")
            } else {
                format!("{prefix}│   ")
            };

            render_tree(&entry.children, &next_prefix, args);
        }
    }
}

fn is_executable(path: &Path, metadata: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        metadata.is_file() && (metadata.permissions().mode() & 0o111 != 0)
    }

    #[cfg(windows)]
    {
        let _ = metadata;

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());

        matches!(
            ext.as_deref(),
            Some("exe") | Some("com") | Some("bat") | Some("cmd") | Some("ps1")
        )
    }
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];

    let mut size = bytes as f64;
    let mut unit = 0;

    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }

    format!("{:.1} {}", size, UNITS[unit])
}

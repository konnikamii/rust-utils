use clap::Parser;
use colored::{Colorize, control};
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant, SystemTime};
use tr::{EntryKind, ScanHandle, ScanOptions, TreeEntry};

#[derive(Parser, Debug)]
struct Args {
    /// Directory to scan
    #[arg(default_value = ".")]
    directory: PathBuf,

    /// Show hidden files
    #[arg(short = 'a', long)]
    all: bool,

    /// Exclude entries with these exact names (comma-delimited); never shown or searched into
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

    /// List folders before files
    #[arg(long)]
    dirs_first: bool,

    /// Show only folders (hide files)
    #[arg(short = 'o', long, conflicts_with = "files_only")]
    folders_only: bool,

    /// Match only files; folders are shown only as parents of matches
    #[arg(long)]
    files_only: bool,

    /// Remove Colors from output
    #[arg(short = 'c', long)]
    clean: bool,

    /// Show entries whose name contains any of these comma-separated substrings (e.g. ".txt,log")
    #[arg(short = 'F', long = "filter", value_delimiter = ',')]
    filter: Vec<String>,

    /// Show files with these comma-separated extensions (e.g. "jpg,png")
    #[arg(short = 'e', long = "ext", value_delimiter = ',')]
    extensions: Vec<String>,

    /// Make -F, -e and -x case-sensitive (default: case-insensitive)
    #[arg(short = 'C', long)]
    case_sensitive: bool,

    /// Show files at least this big (e.g. 500K, 10M, 1.5G)
    #[arg(long, value_parser = parse_size)]
    min_size: Option<u64>,

    /// Show files at most this big (e.g. 500K, 10M, 1.5G)
    #[arg(long, value_parser = parse_size)]
    max_size: Option<u64>,

    /// Show files modified within this time (e.g. 30m, 12h, 7d, 2w)
    #[arg(long, value_parser = parse_age)]
    newer: Option<Duration>,

    /// Show files last modified longer ago than this (e.g. 30m, 12h, 7d, 2w)
    #[arg(long, value_parser = parse_age)]
    older: Option<Duration>,

    /// Stop after this many matching entries
    #[arg(short = 'l', long)]
    limit: Option<usize>,
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
    let now = SystemTime::now();
    let kind = if args.folders_only {
        EntryKind::Folders
    } else if args.files_only {
        EntryKind::Files
    } else {
        EntryKind::Any
    };
    let opts = ScanOptions {
        depth: args.depth,
        all: args.all,
        exclude: args.hide_name.clone(),
        sort: !args.no_sort,
        dirs_first: args.dirs_first,
        case_sensitive: args.case_sensitive,
        name_contains: args.filter.clone(),
        extensions: args.extensions.clone(),
        kind,
        min_size: args.min_size,
        max_size: args.max_size,
        modified_after: args.newer.map(|age| now - age),
        modified_before: args.older.map(|age| now - age),
        limit: args.limit,
        dir_sizes: args.folder_sizes,
        count_children: args.file_count,
        handle: Some(ScanHandle::new()),
    };
    let searching = !args.filter.is_empty()
        || !args.extensions.is_empty()
        || args.files_only
        || args.min_size.is_some()
        || args.max_size.is_some()
        || args.newer.is_some()
        || args.older.is_some();

    // Spinner while scanning; only on a terminal so piped output stays clean.
    let scan_start = Instant::now();
    let spinner_running = Arc::new(AtomicBool::new(true));
    let spinner_handle = io::stdout().is_terminal().then(|| {
        let spinner_flag = spinner_running.clone();
        let progress = opts.handle.clone().unwrap_or_default();
        let clean = args.clean;
        thread::spawn(move || {
            let spinner = ['|', '/', '-', '\\'];
            let mut i: usize = 0;
            let stdout = io::stdout();
            let mut handle = stdout.lock();
            while spinner_flag.load(Ordering::Relaxed) {
                let ch = spinner[i % spinner.len()];
                let elapsed = scan_start.elapsed().as_secs_f64();
                let dir = shorten_middle(&progress.current_dir().display().to_string(), 70);
                let dir = if clean { dir } else { dir.dimmed().to_string() };
                // Clear the line each frame: the folder text changes length.
                let _ = write!(handle, "\r\x1b[2KScanning... {ch} {elapsed:.1}s {dir}");
                let _ = handle.flush();
                i = i.wrapping_add(1);
                thread::sleep(Duration::from_millis(80));
            }
            // clear the spinner line
            let _ = write!(handle, "\r\x1b[2K");
            let _ = handle.flush();
        })
    });

    let result = tr::scan(&args.directory, &opts);

    spinner_running.store(false, Ordering::Relaxed);
    if let Some(handle) = spinner_handle {
        let _ = handle.join();
    }
    let tree = result?;

    let display_path = tree.root.display().to_string();
    let name_display = if !args.clean {
        display_path.blue().bold().to_string()
    } else {
        display_path
    };

    // Print summary for the main folder (where command was invoked) when requested
    let size_str = if args.folder_sizes {
        format!(
            "{} ",
            format!("{:>8}", human_size(tree.total_size)).bright_green()
        )
    } else {
        String::new()
    };
    let count_str = if args.file_count {
        format!(" ({})", tree.child_count)
    } else {
        String::new()
    };
    println!("{}{}{}", size_str, name_display, count_str);

    render_tree(&tree.entries, " ", &args);

    let mut done = format!("\nDone in {:.2}s", scan_start.elapsed().as_secs_f64());
    // Without -S folders count as 0 bytes, so the total would be misleading.
    if args.folder_sizes {
        done.push_str(&format!(" · {} total", human_size(tree.total_size)));
    }
    if searching {
        done.push_str(&format!(" · {} matches", tree.matches().count()));
    }
    if tree.truncated {
        done.push_str(" · limit reached, more exist");
    }
    if args.clean {
        println!("{done}");
    } else {
        println!("{}", done.dimmed());
    }

    Ok(())
}

/// Parses sizes like `123`, `500K`, `10M`, `1.5G`, `2TB`, `4MiB` (1024-based).
fn parse_size(s: &str) -> Result<u64, String> {
    let err = || format!("invalid size '{s}' (e.g. 500K, 10M, 1.5G)");
    let lower = s.trim().to_ascii_lowercase();
    let lower = lower.trim_end_matches('b').trim_end_matches('i');
    let (num, mult) = match lower.chars().last() {
        Some('k') => (&lower[..lower.len() - 1], 1u64 << 10),
        Some('m') => (&lower[..lower.len() - 1], 1 << 20),
        Some('g') => (&lower[..lower.len() - 1], 1 << 30),
        Some('t') => (&lower[..lower.len() - 1], 1 << 40),
        _ => (lower, 1),
    };
    let n: f64 = num.trim().parse().map_err(|_| err())?;
    if !n.is_finite() || n < 0.0 {
        return Err(err());
    }
    Ok((n * mult as f64) as u64)
}

/// Parses ages like `45s`, `30m`, `12h`, `7d`, `2w`.
fn parse_age(s: &str) -> Result<Duration, String> {
    let err = || format!("invalid age '{s}' (e.g. 30m, 12h, 7d, 2w)");
    let t = s.trim();
    let unit = t
        .chars()
        .last()
        .filter(char::is_ascii_alphabetic)
        .ok_or_else(err)?;
    let n: u64 = t[..t.len() - 1].trim().parse().map_err(|_| err())?;
    let secs = match unit.to_ascii_lowercase() {
        's' => 1,
        'm' => 60,
        'h' => 3600,
        'd' => 86_400,
        'w' => 604_800,
        _ => return Err(err()),
    };
    Ok(Duration::from_secs(n.saturating_mul(secs)))
}

/// Fits `s` into `max` characters by replacing its middle with "…", keeping
/// the start (drive / top folders) and the more telling end.
fn shorten_middle(s: &str, max: usize) -> String {
    let len = s.chars().count();
    if len <= max {
        return s.to_string();
    }
    let head_len = (max - 1) / 3;
    let tail_len = max - 1 - head_len;
    let head: String = s.chars().take(head_len).collect();
    let tail: String = s.chars().skip(len - tail_len).collect();
    format!("{head}…{tail}")
}

// Highlight occurrences of filter patterns within a name.
fn highlight_name_with_style<F>(name: &str, args: &Args, base_style: F) -> String
where
    F: Fn(&str) -> colored::ColoredString,
{
    if args.clean {
        return name.to_string();
    }

    let patterns: Vec<&str> = args
        .filter
        .iter()
        .map(String::as_str)
        .filter(|p| !p.is_empty())
        .collect();
    if patterns.is_empty() {
        return base_style(name).to_string();
    }

    let mut out = String::new();
    let mut remaining = name;

    while !remaining.is_empty() {
        // earliest match among patterns
        let best = patterns
            .iter()
            .filter_map(|p| tr::find_pattern(remaining, p, args.case_sensitive))
            .min_by_key(|r| r.start);

        match best {
            Some(range) => {
                if range.start > 0 {
                    out.push_str(&base_style(&remaining[..range.start]).to_string());
                }
                out.push_str(&remaining[range.clone()].red().to_string());
                remaining = &remaining[range.end..];
            }
            None => {
                out.push_str(&base_style(remaining).to_string());
                break;
            }
        }
    }
    out
}

fn render_tree(entries: &[TreeEntry], prefix: &str, args: &Args) {
    let last = entries.len().saturating_sub(1);

    for (i, entry) in entries.iter().enumerate() {
        let is_last = i == last;
        let connector = if is_last { "└── " } else { "├── " };

        let file_name = if entry.is_dir {
            // highlight matches inside directory name but keep directory color for non-matching parts
            format!(
                "{}/",
                highlight_name_with_style(&entry.name, args, |s| s.blue().bold())
            )
        } else if entry.is_exec {
            highlight_name_with_style(&entry.name, args, |s| s.green().bold())
        } else if entry.is_hidden {
            highlight_name_with_style(&entry.name, args, |s| s.dimmed())
        } else {
            highlight_name_with_style(&entry.name, args, |s| s.normal())
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

        println!("{prefix}{connector}{size}{file_name}{child_count}");

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(parse_size("123"), Ok(123));
        assert_eq!(parse_size("500K"), Ok(500 * 1024));
        assert_eq!(parse_size("1.5g"), Ok(1536 * 1024 * 1024));
        assert_eq!(parse_size("4MiB"), Ok(4 << 20));
        assert_eq!(parse_size("10MB"), Ok(10 << 20));
        assert!(parse_size("abc").is_err());
        assert!(parse_size("-1K").is_err());
    }

    #[test]
    fn ages() {
        assert_eq!(parse_age("30m"), Ok(Duration::from_secs(1800)));
        assert_eq!(parse_age("7d"), Ok(Duration::from_secs(7 * 86_400)));
        assert_eq!(parse_age("2W"), Ok(Duration::from_secs(14 * 86_400)));
        assert!(parse_age("7").is_err());
        assert!(parse_age("7x").is_err());
        assert!(parse_age("d").is_err());
    }
}

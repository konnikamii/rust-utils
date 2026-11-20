use clap::Parser;
use colored::{Colorize, control};
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::Instant;
use tr::{ScanOptions, ScanProgress, TreeEntry};

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
    let opts = ScanOptions {
        depth: args.depth,
        all: args.all,
        hide_names: args.hide_name.clone(),
        folders_only: args.folders_only,
        filter: filter_patterns(&args),
        sort: !args.no_sort,
        dir_sizes: args.folder_sizes,
        count_children: args.file_count,
        progress: Some(Arc::new(ScanProgress::default())),
    };

    // Spinner while scanning; only on a terminal so piped output stays clean.
    let scan_start = Instant::now();
    let spinner_running = Arc::new(AtomicBool::new(true));
    let spinner_handle = io::stdout().is_terminal().then(|| {
        let spinner_flag = spinner_running.clone();
        let progress = opts.progress.clone().unwrap_or_default();
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
                thread::sleep(std::time::Duration::from_millis(80));
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
    if args.clean {
        println!("{done}");
    } else {
        println!("{}", done.dimmed());
    }

    Ok(())
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

fn filter_patterns(args: &Args) -> Vec<String> {
    args.filter
        .iter()
        .flat_map(|f| f.split(','))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
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

fn render_tree(entries: &[TreeEntry], prefix: &str, args: &Args) {
    let last = entries.len().saturating_sub(1);

    for (i, entry) in entries.iter().enumerate() {
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
                highlight_name_with_style(&entry.name, args, |s| s.green().bold())
            } else {
                entry.name.clone()
            }
        } else if entry.is_hidden {
            if !args.clean {
                highlight_name_with_style(&entry.name, args, |s| s.dimmed())
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

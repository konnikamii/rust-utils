use clap::Parser;
use colored::Colorize;
use regex::RegexBuilder;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::PathBuf;
use walkdir::WalkDir;

#[derive(Parser, Debug)]
#[command(author, version, about = "Small grep-like recursive search tool", long_about = None)]
struct Args {
    /// Pattern (regular expression)
    pattern: String,

    /// Files or directories to search (defaults to stdin `-`)
    paths: Vec<PathBuf>,

    /// Case-sensitive search (default: case-insensitive)
    #[arg(short = 's', long = "case-sensitive")]
    case_sensitive: bool,

    /// Show line numbers
    #[arg(short = 'n', long = "line-number")]
    line_number: bool,

    /// Recurse into directories (default: true)
    #[arg(short = 'r', long = "recursive", default_value_t = true)]
    recursive: bool,

    /// Filter files by comma-separated substrings in filename (e.g. ".txt,test")
    #[arg(short = 'F', long = "filter")]
    filter: Option<String>,
}

/// Check if the file path matches the filter criteria specified in the arguments.
fn file_matches_filter(path: &PathBuf, args: &Args) -> bool {
    if let Some(ref filt) = args.filter {
        let patterns: Vec<&str> = filt
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        if patterns.is_empty() {
            return true;
        }
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            return patterns.iter().any(|pat| name.contains(pat));
        }
        return false;
    }
    true
}

fn process_file(path: &PathBuf, re: &regex::Regex, args: &Args) -> io::Result<()> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    process_reader(reader, &path.display().to_string(), re, args)
}

fn process_reader<R: BufRead>(
    reader: R,
    path_display: &str,
    re: &regex::Regex,
    args: &Args,
) -> io::Result<()> {
    for (idx, line_res) in reader.lines().enumerate() {
        let line = match line_res {
            Ok(l) => l,
            Err(_) => continue, // skip unreadable lines
        };

        if re.is_match(&line) {
            let highlighted = re.replace_all(&line, |caps: &regex::Captures| {
                format!("{}", &caps[0].red())
            });

            let is_stdin = path_display == "-";
            if is_stdin {
                // When reading from stdin, print only the highlighted line (preserve existing colors)
                println!("{}", highlighted);
            } else {
                let path_str = path_display.bright_black();
                if args.line_number {
                    let line_str = (idx + 1).to_string().bright_black();
                    println!("{}:{}:{}", path_str, line_str, highlighted);
                } else {
                    println!("{}:{}", path_str, highlighted);
                }
            }
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let mut paths = args.paths.clone();
    if paths.is_empty() {
        // If stdin is piped (not a TTY), default to reading from stdin ('-').
        // Otherwise default to current directory.
        if atty::is(atty::Stream::Stdin) {
            paths.push(PathBuf::from("."));
        } else {
            paths.push(PathBuf::from("-"));
        }
    }

    let mut builder = RegexBuilder::new(&args.pattern);
    builder.case_insensitive(!args.case_sensitive);
    let re = builder.build()?;

    for p in paths.iter() {
        if p.is_dir() {
            if args.recursive {
                for entry in WalkDir::new(p).into_iter().filter_map(|e| e.ok()) {
                    let f = entry.path();
                    if f.is_file() {
                        let pb = PathBuf::from(f);
                        if !file_matches_filter(&pb, &args) {
                            continue;
                        }
                        let _ = process_file(&pb, &re, &args);
                    }
                }
            } else {
                // non-recursive: iterate immediate children
                if let Ok(entries) = std::fs::read_dir(p) {
                    for entry in entries.filter_map(|e| e.ok()) {
                        let f = entry.path();
                        if f.is_file() {
                            let pb = PathBuf::from(f);
                            if !file_matches_filter(&pb, &args) {
                                continue;
                            }
                            let _ = process_file(&pb, &re, &args);
                        }
                    }
                }
            }
        } else if p.is_file() {
            if file_matches_filter(p, &args) {
                let _ = process_file(p, &re, &args);
            }
        } else if p.as_os_str() == "-" {
            // read from stdin when path is '-'
            let stdin = io::stdin();
            let _ = process_reader(stdin.lock(), "-", &re, &args);
        } else {
            eprintln!("Skipping unknown path: {}", p.display());
        }
    }

    Ok(())
}

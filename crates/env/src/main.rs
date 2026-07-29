use clap::{Parser, Subcommand};
use colored::Colorize;
use std::env;
use std::error::Error;
use std::fs::{OpenOptions, read_to_string, write};
use std::io::Write;
use std::process::Command;

#[derive(Parser, Debug)]
#[command(name = "env")]
#[command(version, about = "Cli tool to manage environment variables".bold().blue().to_string(), long_about = None)]
#[command()]
struct Args {
    /// Only show variables containing this substring (in key or value)
    #[arg(short = 'F', long)]
    filter: Option<String>,

    /// Load variables from a .env-style file (NAME=VALUE lines)
    #[arg(short = 'f', long = "file")]
    file: Option<String>,

    /// Temporarily set environment variable(s) in NAME=VALUE format (applies only while program runs)
    #[arg(short, long)]
    set: Vec<String>,

    /// Persistently set environment variable(s) in NAME=VALUE format (affects future processes)
    #[arg(short, long)]
    persist: Vec<String>,

    /// Command to run with the modified environment (pass command and its args after `--` e.g., `env -- my_command arg1 arg2`)
    #[arg(trailing_var_arg = true)]
    command: Vec<String>,

    #[command(subcommand)]
    cmd: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Set an environment variable (persisted)
    Set {
        /// Name of the variable
        name: String,
        /// Value of the variable
        value: String,
    },
    /// Append a value to an environment variable (persisted)
    Append {
        /// Name of the variable
        name: String,
        /// Value to append to the variable
        value: String,
    },
    /// Remove an environment variable
    Remove {
        /// Name of the variable
        name: String,
    },
}

fn main() {
    if let Err(e) = run() {
        eprintln!("env: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();

    match &args.cmd {
        Some(Commands::Set { name, value }) => {
            // Persist the variable (platform-specific) and update current process
            if let Err(e) = persist_var(name, value) {
                eprintln!("failed to persist {}: {}", name, e);
                return Err(e);
            }
            println!("Persisted {}={}", name.green(), value);
            return Ok(());
        }
        Some(Commands::Append { name, value }) => {
            // Get current value (if any)
            let current_value = env::var(name).unwrap_or_default();
            let new_value = if current_value.is_empty() {
                value.clone()
            } else {
                format!("{}:{}", current_value, value)
            };
            // Persist the new value (platform-specific) and update current process
            if let Err(e) = persist_var(name, &new_value) {
                eprintln!("failed to persist {}: {}", name, e);
                return Err(e);
            }
            println!("Appended {}={}", name.green(), new_value);
            return Ok(());
        }
        Some(Commands::Remove { name }) => {
            // Remove from persistent storage (platform-specific) and current process
            if let Err(e) = persist_remove(name) {
                eprintln!("failed to remove persisted {}: {}", name, e);
                return Err(e);
            }
            unsafe {
                env::remove_var(name);
            }
            println!("Removed {}", name.green());
            return Ok(());
        }
        None => {}
    }

    // Load variables from .env file if provided
    if let Some(file) = &args.file {
        let content = read_to_string(file)?;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let mut l = trimmed;
            if l.starts_with("export ") {
                l = &l[7..];
            }
            if let Some((k, v)) = parse_name_value(l) {
                unsafe {
                    env::set_var(&k, &v);
                }
            } else {
                eprintln!("warning: ignoring invalid line in {}: {}", file, trimmed);
            }
        }
    }

    // Apply temporary sets first (only affects this process and its children)
    for s in &args.set {
        if let Some((k, v)) = parse_name_value(s) {
            unsafe {
                env::set_var(&k, &v);
            }
        } else {
            eprintln!(
                "warning: ignoring invalid set value '{}', expected NAME=VALUE",
                s
            );
        }
    }

    // Apply persistent sets (platform-specific)
    for p in &args.persist {
        if let Some((k, v)) = parse_name_value(p) {
            if let Err(e) = persist_var(&k, &v) {
                eprintln!("failed to persist {}: {}", k, e);
            }
        } else {
            eprintln!(
                "warning: ignoring invalid persist value '{}', expected NAME=VALUE",
                p
            );
        }
    }

    let mut vars: Vec<(String, String)> = env::vars().collect();
    // Sort case-insensitively so keys like 'VSCODE_...' and 'heyy' order alphabetically
    vars.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));

    let filtered: Vec<(String, String)> = if let Some(f) = args.filter {
        let f_lower = f.to_lowercase();
        vars.into_iter()
            .filter(|(k, v)| {
                k.to_lowercase().contains(&f_lower) || v.to_lowercase().contains(&f_lower)
            })
            .collect()
    } else {
        vars
    };

    for (k, v) in filtered {
        println!("{}={}", k.green(), v);
    }

    // If a command was provided, run it with the modified environment
    if !args.command.is_empty() {
        exec_command(&args.command)?; // this will exit with the child's status
    }

    Ok(())
}

fn parse_name_value(s: &str) -> Option<(String, String)> {
    // Find the first '=' and split there so values may contain '='.
    let idx = match s.find('=') {
        Some(i) => i,
        None => return None,
    };

    let raw_name = s[..idx].trim();
    if raw_name.is_empty() {
        return None;
    }

    // Accept only well-formed variable names: start with a letter or '_',
    // followed by letters, digits, or underscores. Reject malformed names
    // (spaces, dashes, etc.) so lines like "zz2zd 0do-=0 dsa= adsda" are ignored.
    let mut chars = raw_name.chars();
    let first = chars.next().unwrap();
    if !(first.is_ascii_alphabetic() || first == '_') {
        return None;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }

    // Name is already valid; preserve original (trimmed) form.
    let name = raw_name.to_string();

    let raw_value = s[idx + 1..].trim();

    // Handle quoted values (single or double quotes).
    let value = if (raw_value.starts_with('"') && raw_value.ends_with('"'))
        || (raw_value.starts_with('\'') && raw_value.ends_with('\''))
    {
        // Strip surrounding quotes and unescape some common sequences.
        let inner = &raw_value[1..raw_value.len() - 1];
        if raw_value.starts_with('"') {
            inner
                .replace("\\n", "\n")
                .replace("\\\"", "\"")
                .replace("\\\\", "\\")
        } else {
            inner.replace("\\'", "'").replace("\\\\", "\\")
        }
    } else {
        // Unquoted value: take the first token to avoid accidental trailing text
        // (many .env formats either require quotes for spaces or treat rest as value;
        // taking the first token is a pragmatic, conservative choice).
        raw_value
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_string()
    };

    Some((name, value))
}

fn persist_var(name: &str, value: &str) -> Result<(), Box<dyn Error>> {
    // Platform specific persistence
    if cfg!(windows) {
        // Use setx to write user environment variable (visible to new processes)
        let status = Command::new("setx").arg(name).arg(value).status()?;
        if !status.success() {
            return Err(format!("setx failed with status: {}", status).into());
        }
        // Update current process so the new var is visible immediately here
        unsafe {
            env::set_var(name, value);
        }
        Ok(())
    } else {
        // Unix-like: append export to shell rc file
        let home = env::var("HOME").map_err(|_| "HOME not set; cannot persist")?;
        let shell = env::var("SHELL").unwrap_or_default();
        let rc_name = if shell.ends_with("zsh") {
            ".zshrc"
        } else {
            ".bashrc"
        };
        let rc_path = std::path::Path::new(&home).join(rc_name);
        let rc_path = if rc_path.exists() {
            rc_path
        } else {
            std::path::Path::new(&home).join(".profile")
        };

        let mut file = OpenOptions::new().create(true).append(true).open(rc_path)?;
        // Escape single quotes in value for a single-quoted string
        let escaped = value.replace('\'', "'\\''");
        writeln!(file, "export {}='{}'", name, escaped)?;
        // Update current process as well
        unsafe {
            env::set_var(name, value);
        }
        Ok(())
    }
}

fn persist_remove(name: &str) -> Result<(), Box<dyn Error>> {
    if cfg!(windows) {
        // Remove the user environment variable from registry (HKCU\Environment)
        let status = Command::new("reg")
            .arg("delete")
            .arg("HKCU\\Environment")
            .arg("/v")
            .arg(name)
            .arg("/f")
            .status()?;
        if !status.success() {
            return Err(format!("reg delete failed with status: {}", status).into());
        }
        // Remove from current process environment
        unsafe {
            env::remove_var(name);
        }
        Ok(())
    } else {
        // Unix-like: remove export lines from shell rc file if present
        let home = match env::var("HOME") {
            Ok(h) => h,
            Err(_) => return Err("HOME not set; cannot remove persisted variable".into()),
        };
        let shell = env::var("SHELL").unwrap_or_default();
        let rc_name = if shell.ends_with("zsh") {
            ".zshrc"
        } else {
            ".bashrc"
        };
        let mut rc_path = std::path::Path::new(&home).join(rc_name);
        if !rc_path.exists() {
            rc_path = std::path::Path::new(&home).join(".profile");
        }

        if rc_path.exists() {
            let content = read_to_string(&rc_path)?;
            let filtered: Vec<&str> = content
                .lines()
                .filter(|line| {
                    let trimmed = line.trim_start();
                    // Skip lines that start with 'export NAME' (robust-ish)
                    if trimmed.starts_with("export ") {
                        let rest = &trimmed[7..];
                        // match NAME or NAME=...
                        if rest.starts_with(name) {
                            // ensure next char is end, space, or '=' to avoid partial matches
                            return false;
                        }
                    }
                    true
                })
                .collect();
            let new_content = filtered.join("\n");
            write(&rc_path, new_content)?;
        }

        // Remove from current process environment
        unsafe {
            env::remove_var(name);
        }

        Ok(())
    }
}

/// Executes a command with the current environment and exits with its status code.
fn exec_command(cmd: &[String]) -> Result<(), Box<dyn Error>> {
    if cmd.is_empty() {
        return Ok(());
    }
    let program = &cmd[0];
    let args = &cmd[1..];
    // First try to spawn the program directly (for external executables)
    let direct_spawn = (|| -> Result<std::process::ExitStatus, std::io::Error> {
        let mut c = Command::new(program);
        if !args.is_empty() {
            c.args(args);
        }
        c.stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit());
        c.status()
    })();

    let status = match direct_spawn {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Program not found as an external binary; fallback to running via shell
            // Reconstruct a command line from the args so shell builtins work
            let joined = cmd
                .iter()
                .map(|s| {
                    // If an arg contains spaces, quote it so shell preserves it
                    if s.contains(' ') {
                        format!("\"{}\"", s)
                    } else {
                        s.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");

            if cfg!(windows) {
                Command::new("cmd")
                    .arg("/C")
                    .arg(joined)
                    .stdin(std::process::Stdio::inherit())
                    .stdout(std::process::Stdio::inherit())
                    .stderr(std::process::Stdio::inherit())
                    .status()?
            } else {
                Command::new("sh")
                    .arg("-c")
                    .arg(joined)
                    .stdin(std::process::Stdio::inherit())
                    .stdout(std::process::Stdio::inherit())
                    .stderr(std::process::Stdio::inherit())
                    .status()?
            }
        }
        Err(e) => return Err(e.into()),
    };
    let code = status.code().unwrap_or(1);
    std::process::exit(code);
}

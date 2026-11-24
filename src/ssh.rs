use std::io::Write;
use std::process::{Command, Stdio};

use crate::terminfo::{FALLBACK, NAME, SOURCE};

pub const SSH_USAGE: &str = "\
Usage: koral +ssh [--bootstrap-terminfo] [SSH_ARGS...]

Connects via OpenSSH with a compatible TERM fallback (`xterm-256color` when `TERM=xterm-koral`),
or passes `TERM=xterm-koral` and installs `xterm-koral` on the remote host when
`--bootstrap-terminfo` is explicitly passed.
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshPlan {
    pub bootstrap_terminfo: bool,
    pub term: String,
    pub ssh_args: Vec<String>,
    pub destination: Option<String>,
}

fn takes_value(flag: &str) -> bool {
    matches!(
        flag,
        "-B" | "-b"
            | "-c"
            | "-D"
            | "-E"
            | "-e"
            | "-F"
            | "-I"
            | "-i"
            | "-J"
            | "-L"
            | "-l"
            | "-m"
            | "-O"
            | "-o"
            | "-p"
            | "-Q"
            | "-R"
            | "-S"
            | "-W"
            | "-w"
    )
}

pub fn plan(args: &[String], current_term: Option<&str>) -> Result<SshPlan, String> {
    let mut bootstrap_terminfo = false;
    let mut ssh_args = Vec::with_capacity(args.len());
    let mut end_of_wrapper_opts = false;

    for arg in args {
        if !end_of_wrapper_opts {
            if arg == "--" {
                end_of_wrapper_opts = true;
                ssh_args.push(arg.clone());
                continue;
            }
            if arg == "--bootstrap-terminfo" {
                bootstrap_terminfo = true;
                continue;
            }
            if arg == "--help" || arg == "-h" {
                return Err(SSH_USAGE.trim_end().to_string());
            }
        }
        ssh_args.push(arg.clone());
    }

    if ssh_args.is_empty() {
        return Err(SSH_USAGE.trim_end().to_string());
    }

    let destination = extract_destination(&ssh_args);
    if bootstrap_terminfo && destination.is_none() {
        return Err("missing ssh destination for '--bootstrap-terminfo'".to_string());
    }

    let term = if bootstrap_terminfo {
        NAME.to_string()
    } else {
        match current_term {
            None | Some("") | Some(NAME) => FALLBACK.to_string(),
            Some(other) => other.to_string(),
        }
    };

    Ok(SshPlan {
        bootstrap_terminfo,
        term,
        ssh_args,
        destination,
    })
}

fn extract_destination(ssh_args: &[String]) -> Option<String> {
    let mut i = 0;
    let mut past_options = false;
    while i < ssh_args.len() {
        let arg = &ssh_args[i];
        if !past_options && arg == "--" {
            past_options = true;
            i += 1;
            continue;
        }
        if !past_options && arg.starts_with('-') && arg != "-" {
            if arg.len() == 2 && takes_value(arg) {
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        return Some(arg.clone());
    }
    None
}

fn transport_args(ssh_args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < ssh_args.len() {
        let arg = &ssh_args[i];
        if arg == "--" || !arg.starts_with('-') || arg == "-" {
            break;
        }
        let prefix = arg.get(..2).unwrap_or("");
        if matches!(prefix, "-p" | "-i" | "-F" | "-J" | "-l" | "-o") {
            out.push(arg.clone());
            if arg.len() == 2
                && let Some(val) = ssh_args.get(i + 1)
            {
                out.push(val.clone());
                i += 2;
                continue;
            }
        } else if matches!(arg.as_str(), "-4" | "-6" | "-C") {
            out.push(arg.clone());
        } else if arg.len() == 2 && takes_value(arg) {
            i += 2;
            continue;
        }
        i += 1;
    }
    out
}

pub fn run(args: &[String]) -> Result<i32, String> {
    if args.len() == 1 && matches!(args[0].as_str(), "--help" | "-h") {
        print!("{SSH_USAGE}");
        return Ok(0);
    }
    let current_term = std::env::var("TERM").ok();
    let plan = plan(args, current_term.as_deref())?;

    if plan.bootstrap_terminfo {
        let dest = plan
            .destination
            .as_deref()
            .ok_or_else(|| "missing ssh destination for '--bootstrap-terminfo'".to_string())?;
        let mut child = Command::new("ssh")
            .args(transport_args(&plan.ssh_args))
            .arg("--")
            .arg(dest)
            .arg("mkdir -p ~/.terminfo && tic -x -")
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|err| format!("failed to launch ssh for terminfo bootstrap: {err}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(SOURCE.as_bytes())
                .map_err(|err| format!("failed to pipe terminfo to ssh: {err}"))?;
        }
        let status = child
            .wait()
            .map_err(|err| format!("ssh terminfo bootstrap failed: {err}"))?;
        if !status.success() {
            return Err(format!(
                "remote terminfo bootstrap exited with status {}",
                status.code().unwrap_or(1)
            ));
        }
    }

    let status = Command::new("ssh")
        .env("TERM", &plan.term)
        .args(&plan.ssh_args)
        .status()
        .map_err(|err| format!("failed to launch ssh: {err}"))?;
    Ok(status.code().unwrap_or(1))
}

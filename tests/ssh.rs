use koral::config::{Launch, parse_args};
use koral::ssh::{SshPlan, plan};
use koral::terminfo::{FALLBACK, NAME};

fn s(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_string()).collect()
}

#[test]
fn xterm_koral_falls_back_to_xterm_256color_without_bootstrap() {
    let args = s(&["user@example.com"]);
    assert_eq!(
        plan(&args, Some(NAME)).unwrap(),
        SshPlan {
            bootstrap_terminfo: false,
            term: FALLBACK.to_string(),
            ssh_args: args.clone(),
            destination: Some("user@example.com".to_string()),
        }
    );
    assert_eq!(plan(&args, None).unwrap().term, FALLBACK);
}

#[test]
fn preserves_explicit_non_koral_term() {
    let args = s(&["host"]);
    assert_eq!(
        plan(&args, Some("tmux-256color")).unwrap().term,
        "tmux-256color"
    );
}

#[test]
fn bootstrap_terminfo_strips_flag_and_keeps_xterm_koral() {
    let args = s(&[
        "--bootstrap-terminfo",
        "-p",
        "2222",
        "-i",
        "~/.ssh/id_ed25519",
        "ops@host",
        "htop",
    ]);
    assert_eq!(
        plan(&args, Some(FALLBACK)).unwrap(),
        SshPlan {
            bootstrap_terminfo: true,
            term: NAME.to_string(),
            ssh_args: s(&["-p", "2222", "-i", "~/.ssh/id_ed25519", "ops@host", "htop"]),
            destination: Some("ops@host".to_string()),
        }
    );
}

#[test]
fn skips_option_values_when_resolving_destination_and_rejects_missing_args() {
    let args = s(&["-o", "StrictHostKeyChecking=no", "-p2200", "box"]);
    assert_eq!(
        plan(&args, Some(NAME)).unwrap().destination.as_deref(),
        Some("box")
    );

    assert!(plan(&[], Some(NAME)).is_err());
    assert!(plan(&s(&["--bootstrap-terminfo", "-p", "22"]), Some(NAME)).is_err());
}

#[test]
fn parse_args_forwards_plus_ssh_arguments_verbatim() {
    assert_eq!(
        parse_args(["+ssh", "--bootstrap-terminfo", "-p", "2222", "user@host"]),
        Ok(Launch::Ssh(s(&[
            "--bootstrap-terminfo",
            "-p",
            "2222",
            "user@host"
        ])))
    );
}

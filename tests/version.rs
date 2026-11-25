use std::path::PathBuf;
use std::process::Command;

use koral::config::{Args, Launch, parse_args};

#[test]
fn version_flag_prints_the_crate_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_koral"))
        .arg("--version")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("koral {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn help_flags_print_the_usage_without_opening_a_window() {
    for flag in ["--help", "-h"] {
        let output = Command::new(env!("CARGO_BIN_EXE_koral"))
            .arg(flag)
            .env_remove("WAYLAND_DISPLAY")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{flag} exited with {:?}",
            output.status
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("Usage: koral"), "{flag}: {stdout}");
        assert!(stdout.contains("--version"), "{flag}: {stdout}");
    }
}

#[test]
fn an_unknown_argument_is_reported_and_fails() {
    let output = Command::new(env!("CARGO_BIN_EXE_koral"))
        .arg("--nope")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown argument '--nope'"), "{stderr}");
    assert!(output.stdout.is_empty());
}

#[test]
fn parses_launch_options_and_command() {
    assert_eq!(parse_args::<_, &str>([]), Ok(Launch::Run(Args::default())));
    assert_eq!(parse_args(["--version"]), Ok(Launch::Version));
    assert_eq!(parse_args(["-h"]), Ok(Launch::Help));
    assert_eq!(parse_args(["--help"]), Ok(Launch::Help));

    let parsed = parse_args([
        "--cwd",
        "/tmp/work",
        "--title",
        "MyTerm",
        "--class",
        "dev.koral",
        "-e",
        "btop",
        "--utf-8",
    ]);
    assert_eq!(
        parsed,
        Ok(Launch::Run(Args {
            cwd: Some(PathBuf::from("/tmp/work")),
            title: Some("MyTerm".to_string()),
            class: Some("dev.koral".to_string()),
            command: Some(("btop".to_string(), vec!["--utf-8".to_string()])),
        }))
    );

    let equals = parse_args([
        "--working-directory=/var/log",
        "--app-id=float-term",
        "--title=Top",
        "--",
        "sh",
        "-c",
        "hi",
    ]);
    assert_eq!(
        equals,
        Ok(Launch::Run(Args {
            cwd: Some(PathBuf::from("/var/log")),
            title: Some("Top".to_string()),
            class: Some("float-term".to_string()),
            command: Some(("sh".to_string(), vec!["-c".to_string(), "hi".to_string()])),
        }))
    );

    for bad in [
        vec!["--cwd"],
        vec!["--title"],
        vec!["--class"],
        vec!["-e"],
        vec!["--command"],
        vec!["--"],
        vec!["--unknown"],
    ] {
        assert!(parse_args(bad).is_err());
    }
    assert_eq!(
        parse_args(["ctl", "--socket", "/tmp/k.sock", "list"]),
        Ok(Launch::Control(vec![
            "--socket".into(),
            "/tmp/k.sock".into(),
            "list".into()
        ]))
    );
    assert_eq!(
        parse_args(["@-1"]),
        Ok(Launch::BlockOutput {
            selector: "@-1".into(),
            pane: None,
            json: false,
        })
    );
    assert_eq!(
        parse_args(["@err", "--pane", "3", "--json"]),
        Ok(Launch::BlockOutput {
            selector: "@err".into(),
            pane: Some(3),
            json: true,
        })
    );
    assert!(parse_args(["@-1", "--bogus"]).is_err());
}

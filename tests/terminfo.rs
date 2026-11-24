use std::path::PathBuf;
use std::process::Command;

use koral::config::{Launch, parse_args};
use koral::terminfo::{FALLBACK, NAME, SOURCE, installed, resolve};

fn scratch(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("koral-terminfo-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn installed_checks_both_letter_and_hex_layouts() {
    let empty = scratch("empty");
    assert!(!installed(std::slice::from_ref(&empty)));
    assert_eq!(resolve(std::slice::from_ref(&empty)), FALLBACK);

    let letter = scratch("letter");
    std::fs::create_dir_all(letter.join("x")).unwrap();
    std::fs::write(letter.join("x/xterm-koral"), b"stub").unwrap();
    assert!(installed(std::slice::from_ref(&letter)));
    assert_eq!(resolve(std::slice::from_ref(&letter)), NAME);

    let hex = scratch("hex");
    std::fs::create_dir_all(hex.join("78")).unwrap();
    std::fs::write(hex.join("78/xterm-koral"), b"stub").unwrap();
    assert!(installed(std::slice::from_ref(&hex)));
    assert_eq!(resolve(std::slice::from_ref(&hex)), NAME);
}

#[test]
fn parse_args_recognizes_terminfo_install() {
    assert_eq!(
        parse_args(["+terminfo", "install"]),
        Ok(Launch::TerminfoInstall)
    );
    assert!(parse_args(["+terminfo"]).is_err());
    assert!(parse_args(["+terminfo", "nope"]).is_err());
    assert!(parse_args(["+terminfo", "install", "extra"]).is_err());
}

#[test]
fn terminfo_source_compiles_with_tic() {
    if !Command::new("which")
        .arg("tic")
        .output()
        .is_ok_and(|out| out.status.success())
    {
        eprintln!("skipping tic test: tic not installed");
        return;
    }

    let dir = scratch("tic");
    let src = dir.join("xterm-koral.terminfo");
    let out_dir = dir.join("out");
    std::fs::write(&src, SOURCE).unwrap();
    std::fs::create_dir_all(&out_dir).unwrap();

    let output = Command::new("tic")
        .arg("-x")
        .arg("-o")
        .arg(&out_dir)
        .arg(&src)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "tic failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(installed(&[out_dir]));
}

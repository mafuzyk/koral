use std::process::Command;

#[test]
fn version_flag_prints_the_crate_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_koral")).arg("--version").env_remove("WAYLAND_DISPLAY").output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), format!("koral {}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn help_flags_print_the_usage_without_opening_a_window() {
    for flag in ["--help", "-h"] {
        let output = Command::new(env!("CARGO_BIN_EXE_koral")).arg(flag).env_remove("WAYLAND_DISPLAY").output().unwrap();
        assert!(output.status.success(), "{flag} exited with {:?}", output.status);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("Usage: koral"), "{flag}: {stdout}");
        assert!(stdout.contains("--version"), "{flag}: {stdout}");
    }
}

#[test]
fn an_unknown_argument_is_reported_and_fails() {
    let output =
        Command::new(env!("CARGO_BIN_EXE_koral")).arg("--nope").env_remove("WAYLAND_DISPLAY").output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown argument '--nope'"), "{stderr}");
    assert!(output.stdout.is_empty());
}

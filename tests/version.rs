use std::process::Command;

#[test]
fn version_flag_prints_the_crate_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_koral")).arg("--version").env_remove("WAYLAND_DISPLAY").output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), format!("koral {}\n", env!("CARGO_PKG_VERSION")));
}

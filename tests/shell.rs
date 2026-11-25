use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::event::WindowSize;
use alacritty_terminal::tty::{
    ChildEvent, EventedPty, EventedReadWrite, Options, Shell, new as new_pty,
};
use koral::marks::{Mark, Piece, Scanner};
use koral::shell::{BASH_SCRIPT, FISH_SCRIPT, ZSH_SCRIPT, plan, prepare};
use polling::{Event as PollEvent, Events, PollMode, Poller};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("koral-shell-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn plan_covers_supported_and_passthrough_shells() {
    let dir = PathBuf::from("/run/user/1000/koral/shell-integration");
    let empty = |_: &str| None;

    let bash = plan("/usr/bin/bash", &empty, &dir);
    assert_eq!(
        bash.args,
        vec![
            "--rcfile",
            "/run/user/1000/koral/shell-integration/koral.bash"
        ]
    );
    assert!(bash.env.is_empty());

    let zsh_plain = plan("zsh", &empty, &dir);
    assert!(zsh_plain.args.is_empty());
    assert_eq!(
        zsh_plain.env,
        vec![(
            "ZDOTDIR".to_string(),
            "/run/user/1000/koral/shell-integration/zsh".to_string()
        )]
    );

    let with_zdotdir = |key: &str| match key {
        "ZDOTDIR" => Some("/home/alice/.config/zsh".to_string()),
        _ => None,
    };
    let zsh_custom = plan("/bin/zsh", &with_zdotdir, &dir);
    assert_eq!(
        zsh_custom.env,
        vec![
            (
                "ZDOTDIR".to_string(),
                "/run/user/1000/koral/shell-integration/zsh".to_string()
            ),
            (
                "KORAL_ZSH_ZDOTDIR".to_string(),
                "/home/alice/.config/zsh".to_string()
            ),
        ]
    );

    let fish_plain = plan("/usr/bin/fish", &empty, &dir);
    assert!(fish_plain.args.is_empty());
    assert_eq!(
        fish_plain.env,
        vec![(
            "XDG_DATA_DIRS".to_string(),
            "/run/user/1000/koral/shell-integration/fish-data:/usr/local/share:/usr/share"
                .to_string()
        )]
    );

    let with_xdg = |key: &str| match key {
        "XDG_DATA_DIRS" => Some("/custom/share".to_string()),
        _ => None,
    };
    let fish_custom = plan("fish", &with_xdg, &dir);
    assert_eq!(
        fish_custom.env,
        vec![(
            "XDG_DATA_DIRS".to_string(),
            "/run/user/1000/koral/shell-integration/fish-data:/custom/share".to_string()
        )]
    );

    assert!(plan("sharky", &empty, &dir).args.is_empty());
    assert!(plan("sharky", &empty, &dir).env.is_empty());
    assert!(plan("/bin/sh", &empty, &dir).args.is_empty());
    assert!(plan("/bin/sh", &empty, &dir).env.is_empty());
}

#[test]
fn prepare_writes_0700_dirs_and_is_idempotent() {
    let dir = scratch("prepare");
    prepare(&dir).unwrap();

    let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);
    assert_eq!(
        std::fs::read_to_string(dir.join("koral.bash")).unwrap(),
        BASH_SCRIPT
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("zsh/.zshenv")).unwrap(),
        ZSH_SCRIPT
    );
    assert_eq!(
        std::fs::read_to_string(
            dir.join("fish-data/fish/vendor_conf.d/koral-shell-integration.fish")
        )
        .unwrap(),
        FISH_SCRIPT
    );

    prepare(&dir).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("koral.bash")).unwrap(),
        BASH_SCRIPT
    );
}

#[test]
fn scripts_emit_expected_marks_without_modifying_user_files() {
    for (name, script) in [
        ("bash", BASH_SCRIPT),
        ("zsh", ZSH_SCRIPT),
        ("fish", FISH_SCRIPT),
    ] {
        assert!(script.contains("133;A"), "{name} missing 133;A");
        assert!(script.contains("133;C"), "{name} missing 133;C");
        assert!(script.contains("133;D"), "{name} missing 133;D");
        assert!(script.contains("]7;"), "{name} missing OSC 7");
        assert!(!script.contains(">>"), "{name} contains append redirect");
        assert!(!script.contains("rm "), "{name} contains rm");
        assert!(!script.contains("$HOME/"), "{name} writes under $HOME");
    }
}

#[test]
fn live_bash_integration_emits_prompt_command_status_and_cwd_marks() {
    let dir = scratch("live-bash");
    let home = dir.join("home");
    let runtime = dir.join("runtime");
    std::fs::create_dir_all(&home).unwrap();
    prepare(&runtime).unwrap();

    let rcfile = runtime.join("koral.bash");
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(r#"exec bash --rcfile "$1" -i 2>&1"#)
        .arg("sh")
        .arg(&rcfile)
        .env("HOME", &home)
        .env_remove("KORAL_SHELL_INTEGRATION")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn interactive bash");

    {
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(b"true\nfalse\n\ncd /tmp\nexit\n").unwrap();
    }
    let output = child.wait_with_output().unwrap();
    let mut scanner = Scanner::default();
    let mut marks = Vec::new();
    scanner.feed(&output.stdout, &mut |piece| {
        if let Piece::Mark(mark) = piece {
            marks.push(mark);
        }
    });

    let cmd_ends: Vec<Option<i32>> = marks
        .iter()
        .filter_map(|mark| match mark {
            Mark::CommandEnd(status) => Some(*status),
            _ => None,
        })
        .collect();
    assert_eq!(
        cmd_ends,
        vec![Some(0), Some(1), Some(0)],
        "marks: {marks:?}"
    );
    assert!(marks.contains(&Mark::PromptStart));
    assert!(marks.contains(&Mark::PromptEnd));
    assert!(marks.contains(&Mark::CommandStart(None)));
    assert!(marks.contains(&Mark::Cwd("/tmp".to_string())));

    // Check PromptStart is never duplicated consecutively without an intervening PromptEnd/Cwd.
    let mut seen_start = false;
    for mark in &marks {
        match mark {
            Mark::PromptStart => {
                assert!(!seen_start, "duplicate PromptStart: {marks:?}");
                seen_start = true;
            }
            Mark::CommandStart(_) | Mark::PromptEnd => seen_start = false,
            _ => {}
        }
    }
}

fn contains_tail_subslice(buf: &[u8], prev_len: usize, needle: &[u8]) -> bool {
    let start = prev_len.saturating_sub(needle.len().saturating_sub(1));
    buf[start..].windows(needle.len()).any(|w| w == needle)
}

fn run_interactive_pty(program: &str, home: &Path, extra_env: HashMap<String, String>) -> Vec<u8> {
    let mut env = extra_env;
    env.insert("HOME".to_string(), home.display().to_string());
    env.insert("TERM".to_string(), "xterm-256color".to_string());

    let options = Options {
        shell: Some(Shell::new(
            "env".to_string(),
            vec![
                "-u".to_string(),
                "KORAL_SHELL_INTEGRATION".to_string(),
                program.to_string(),
                "-i".to_string(),
            ],
        )),
        working_directory: Some(home.to_path_buf()),
        drain_on_exit: false,
        env,
    };
    let size = WindowSize {
        num_lines: 24,
        num_cols: 80,
        cell_width: 8,
        cell_height: 16,
    };
    let mut pty = new_pty(&options, size, 0).expect("spawn shell PTY");
    let mut writer = pty.file().try_clone().expect("clone PTY master");
    let poller = Arc::new(Poller::new().expect("create poller"));
    // SAFETY: `pty` is deregistered before it drops at the end of this function.
    unsafe {
        pty.register(&poller, PollEvent::readable(0), PollMode::Level)
            .expect("register PTY with poller");
    }

    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    let mut events = Events::new();
    let mut sent_commands = false;
    let deadline = Instant::now() + Duration::from_secs(5);

    'outer: while Instant::now() < deadline {
        events.clear();
        let remaining = deadline.saturating_duration_since(Instant::now());
        let _ = poller.wait(&mut events, Some(remaining.min(Duration::from_millis(100))));

        let exited = matches!(pty.next_child_event(), Some(ChildEvent::Exited(_)));
        let mut eof = false;
        loop {
            match pty.reader().read(&mut buf) {
                Ok(0) => {
                    eof = true;
                    break;
                }
                Ok(n) => {
                    let prev_len = out.len();
                    out.extend_from_slice(&buf[..n]);
                    if contains_tail_subslice(&out, prev_len, b"\x1b[6n") {
                        let _ = writer.write_all(b"\x1b[1;1R");
                    }
                    if contains_tail_subslice(&out, prev_len, b"\x1b[0c")
                        || contains_tail_subslice(&out, prev_len, b"\x1b[c")
                    {
                        let _ = writer.write_all(b"\x1b[?62;22c");
                    }
                    if !sent_commands && out.windows(5).any(|w| w == b"133;B") {
                        writer
                            .write_all(b"true\nfalse\nexit\n")
                            .expect("write shell smoke commands");
                        sent_commands = true;
                    }
                }
                Err(err) if err.kind() == ErrorKind::Interrupted => continue,
                Err(err) if err.kind() == ErrorKind::WouldBlock => break,
                Err(_) => {
                    eof = true;
                    break;
                }
            }
        }

        if eof || exited {
            break 'outer;
        }
        if pty
            .reregister(&poller, PollEvent::readable(0), PollMode::Level)
            .is_err()
        {
            break 'outer;
        }
    }

    let _ = pty.deregister(&poller);
    out
}

fn assert_live_shell(program: &str, home: &Path, runtime: &Path) {
    if !Command::new("which")
        .arg(program)
        .output()
        .is_ok_and(|out| out.status.success())
    {
        eprintln!("skipping live {program} test: {program} not installed");
        return;
    }

    let empty = |_: &str| None;
    let inj = plan(program, &empty, runtime);
    let env: HashMap<String, String> = inj.env.into_iter().collect();
    let out = run_interactive_pty(program, home, env);
    let mut scanner = Scanner::default();
    let mut marks = Vec::new();
    scanner.feed(&out, &mut |p| {
        if let Piece::Mark(m) = p {
            marks.push(m);
        }
    });
    assert!(
        marks.contains(&Mark::CommandEnd(Some(0))),
        "{program} marks: {marks:?}"
    );
    assert!(
        marks.contains(&Mark::CommandEnd(Some(1))),
        "{program} marks: {marks:?}"
    );
}

#[test]
fn live_zsh_and_fish_smoke_when_available() {
    let dir = scratch("live-others");
    let home = dir.join("home");
    let runtime = dir.join("runtime");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join(".zshrc"), b"").unwrap();
    prepare(&runtime).unwrap();

    assert_live_shell("zsh", &home, &runtime);
    assert_live_shell("fish", &home, &runtime);
}

use std::ffi::CString;
use std::io::{ErrorKind, Read};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use winit::event_loop::EventLoopProxy;

use crate::config;
use crate::pty::UserEvent;
use crate::theme::Theme;

const IN_CLOEXEC: i32 = 0o2000000;
// IN_MODIFY | IN_ATTRIB | IN_CLOSE_WRITE | IN_MOVED_TO | IN_CREATE
const WATCH_MASK: u32 = 0x0000_0002 | 0x0000_0004 | 0x0000_0008 | 0x0000_0080 | 0x0000_0100;

unsafe extern "C" {
    fn inotify_init1(flags: i32) -> i32;
    fn inotify_add_watch(fd: i32, pathname: *const std::ffi::c_char, mask: u32) -> i32;
}

pub const TEMPLATE: &str = include_str!("../assets/koral.matugen");

fn colors_path() -> Option<PathBuf> {
    Some(config::home()?.join("koral/matugen.colors"))
}

pub fn theme(enabled: bool) -> Theme {
    let colors = colors_path().filter(|_| enabled).and_then(|path| std::fs::read_to_string(path).ok());
    colors.map_or_else(Theme::sharky, |colors| Theme::with_colors(&colors))
}

/// Matugen's config text with Koral's template in it, or None when it is already there.
pub fn add_template(source: &str, template: &Path, output: &Path) -> Option<String> {
    if source.contains("[templates.koral]") {
        return None;
    }
    let mut updated = source.to_string();
    if !updated.is_empty() {
        if !updated.ends_with('\n') {
            updated.push('\n');
        }
        updated.push('\n');
    }
    updated.push_str(&format!("[templates.koral]\ninput_path = {template:?}\noutput_path = {output:?}\n"));
    Some(updated)
}

/// Whether a usable `matugen` is installed: the toggle has nothing to run without it.
pub fn on_path(path: Option<&std::ffi::OsStr>) -> bool {
    use std::os::unix::fs::PermissionsExt;

    let Some(path) = path else {
        return false;
    };
    std::env::split_paths(path).any(|dir| {
        std::fs::metadata(dir.join("matugen")).is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    })
}

/// Makes the next matugen run write Koral's colours.
pub fn install() -> Result<(), String> {
    if !on_path(std::env::var_os("PATH").as_deref()) {
        return Err("matugen is not installed".to_string());
    }
    let home = config::home().ok_or("Not saved: no config folder")?;
    // Dotfile managers link this file: writing through the link keeps it theirs.
    let matugen = std::fs::canonicalize(home.join("matugen/config.toml")).map_err(|_| "No matugen config found")?;
    let source = std::fs::read_to_string(&matugen).map_err(|error| format!("Cannot read matugen config: {error}"))?;

    let template = home.join("koral/matugen.template");
    let write = |error: std::io::Error| format!("Not saved: {error}");
    config::save(&template, TEMPLATE).map_err(write)?;
    if let Some(updated) = add_template(&source, &template, &home.join("koral/matugen.colors")) {
        config::save(&matugen, &updated).map_err(write)?;
    }
    Ok(())
}

fn modified() -> [Option<SystemTime>; 3] {
    let time = |path: Option<PathBuf>| std::fs::metadata(path?).ok()?.modified().ok();
    [
        time(colors_path()),
        time(config::home().map(|home| home.join("sharky/theme.sharky"))),
        time(config::path()),
    ]
}

pub fn diff_stamps(before: &[Option<SystemTime>; 3], after: &[Option<SystemTime>; 3]) -> (bool, bool) {
    let colors = before[0] != after[0] || before[1] != after[1];
    let config = before[2] != after[2];
    (colors, config)
}

fn add_watches(fd: i32) {
    let Some(home) = config::home() else {
        return;
    };
    for dir in [&home, &home.join("koral"), &home.join("sharky")] {
        if let Ok(path) = CString::new(dir.as_os_str().as_bytes()) {
            unsafe { inotify_add_watch(fd, path.as_ptr(), WATCH_MASK) };
        }
    }
}

fn notify_changes(proxy: &EventLoopProxy<UserEvent>, seen: &mut [Option<SystemTime>; 3]) -> bool {
    let now = modified();
    let (colors, config) = diff_stamps(seen, &now);
    *seen = now;
    if colors && proxy.send_event(UserEvent::Colors).is_err() {
        return false;
    }
    if config && proxy.send_event(UserEvent::Config).is_err() {
        return false;
    }
    true
}

pub fn watch(proxy: EventLoopProxy<UserEvent>) {
    std::thread::spawn(move || {
        let mut seen = modified();
        let fd = unsafe { inotify_init1(IN_CLOEXEC) };
        if fd >= 0 {
            // SAFETY: `fd` is a valid owned file descriptor from `inotify_init1`.
            let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
            add_watches(file.as_raw_fd());
            let mut buffer = [0u8; 4096];
            loop {
                match file.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(_) => {
                        add_watches(file.as_raw_fd());
                        if !notify_changes(&proxy, &mut seen) {
                            return;
                        }
                    }
                    Err(error) if error.kind() == ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
        }
        loop {
            std::thread::sleep(Duration::from_secs(1));
            if !notify_changes(&proxy, &mut seen) {
                return;
            }
        }
    });
}

use std::fs;
use std::path::PathBuf;
use std::process::Command;

pub const NAME: &str = "xterm-koral";
pub const FALLBACK: &str = "xterm-256color";
pub const SOURCE: &str = include_str!("../assets/xterm-koral.terminfo");

/// Directories searched for compiled terminfo entries, without spawning `infocmp` at startup.
pub fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = std::env::var_os("TERMINFO")
        && !dir.is_empty()
    {
        dirs.push(PathBuf::from(dir));
    }
    if let Some(home) = std::env::var_os("HOME")
        && !home.is_empty()
    {
        dirs.push(PathBuf::from(home).join(".terminfo"));
    }
    if let Some(list) = std::env::var_os("TERMINFO_DIRS") {
        for entry in std::env::split_paths(&list) {
            if entry.as_os_str().is_empty() {
                dirs.push(PathBuf::from("/usr/share/terminfo"));
            } else {
                dirs.push(entry);
            }
        }
    }
    for system in ["/usr/share/terminfo", "/etc/terminfo", "/lib/terminfo"] {
        let path = PathBuf::from(system);
        if !dirs.contains(&path) {
            dirs.push(path);
        }
    }
    dirs
}

/// Checks both ncurses letter (`x/xterm-koral`) and hex (`78/xterm-koral`) directory layouts.
pub fn installed(dirs: &[PathBuf]) -> bool {
    dirs.iter()
        .any(|dir| dir.join("x").join(NAME).is_file() || dir.join("78").join(NAME).is_file())
}

pub fn resolve(dirs: &[PathBuf]) -> &'static str {
    if installed(dirs) { NAME } else { FALLBACK }
}

/// Compiles [`SOURCE`] into `~/.terminfo` using `tic`.
pub fn install() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME")
        .filter(|val| !val.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set".to_string())?;
    let target = home.join(".terminfo");
    fs::create_dir_all(&target)
        .map_err(|err| format!("cannot create {}: {err}", target.display()))?;

    let tmp = std::env::temp_dir().join(format!("xterm-koral-{}.terminfo", std::process::id()));
    fs::write(&tmp, SOURCE).map_err(|err| format!("cannot write {}: {err}", tmp.display()))?;

    let output = Command::new("tic")
        .arg("-x")
        .arg("-o")
        .arg(&target)
        .arg(&tmp)
        .output();
    let _ = fs::remove_file(&tmp);

    let output = output.map_err(|err| format!("cannot run tic: {err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            format!("tic exited with {}", output.status)
        } else {
            stderr
        };
        return Err(detail);
    }
    Ok(target)
}

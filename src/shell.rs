use std::fs::{self, Permissions};
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

pub const BASH_SCRIPT: &str = include_str!("../assets/shell/koral.bash");
pub const ZSH_SCRIPT: &str = include_str!("../assets/shell/zsh/.zshenv");
pub const FISH_SCRIPT: &str =
    include_str!("../assets/shell/fish/vendor_conf.d/koral-shell-integration.fish");

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Injection {
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// Plans shell-specific integration args and environment overrides without touching user rc files.
pub fn plan(shell: &str, env: &impl Fn(&str) -> Option<String>, dir: &Path) -> Injection {
    let name = Path::new(shell)
        .file_name()
        .and_then(|part| part.to_str())
        .unwrap_or(shell);

    match name {
        "bash" => Injection {
            args: vec![
                "--rcfile".to_string(),
                dir.join("koral.bash").display().to_string(),
            ],
            env: Vec::new(),
        },
        "zsh" => {
            let mut vars = vec![("ZDOTDIR".to_string(), dir.join("zsh").display().to_string())];
            if let Some(original) = env("ZDOTDIR") {
                vars.push(("KORAL_ZSH_ZDOTDIR".to_string(), original));
            }
            Injection {
                args: Vec::new(),
                env: vars,
            }
        }
        "fish" => {
            let fish_data = dir.join("fish-data").display().to_string();
            let base = env("XDG_DATA_DIRS")
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "/usr/local/share:/usr/share".to_string());
            Injection {
                args: Vec::new(),
                env: vec![("XDG_DATA_DIRS".to_string(), format!("{fish_data}:{base}"))],
            }
        }
        _ => Injection::default(),
    }
}

/// Writes the embedded integration scripts under `dir` with 0700 directory permissions.
pub fn prepare(dir: &Path) -> io::Result<()> {
    let zsh_dir = dir.join("zsh");
    let fish_dir = dir.join("fish-data/fish/vendor_conf.d");
    for folder in [
        dir,
        zsh_dir.as_path(),
        dir.join("fish-data").as_path(),
        dir.join("fish-data/fish").as_path(),
        fish_dir.as_path(),
    ] {
        fs::create_dir_all(folder)?;
        fs::set_permissions(folder, Permissions::from_mode(0o700))?;
    }

    write_if_changed(&dir.join("koral.bash"), BASH_SCRIPT)?;
    write_if_changed(&zsh_dir.join(".zshenv"), ZSH_SCRIPT)?;
    write_if_changed(&fish_dir.join("koral-shell-integration.fish"), FISH_SCRIPT)?;
    Ok(())
}

fn write_if_changed(path: &Path, content: &str) -> io::Result<()> {
    if let Ok(existing) = fs::read_to_string(path)
        && existing == content
    {
        return Ok(());
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, content)?;
    fs::set_permissions(&tmp, Permissions::from_mode(0o600))?;
    fs::rename(&tmp, path)
}

pub fn dir() -> PathBuf {
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR")
        && !runtime.is_empty()
    {
        return PathBuf::from(runtime).join("koral/shell-integration");
    }
    let uid = std::env::var_os("HOME")
        .and_then(|home| fs::metadata(home).ok())
        .map(|meta| meta.uid().to_string())
        .or_else(|| std::env::var("USER").ok().filter(|user| !user.is_empty()))
        .unwrap_or_else(|| "user".to_string());
    std::env::temp_dir().join(format!("koral-shell-{uid}"))
}

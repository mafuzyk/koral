use std::io::Write;
use std::path::PathBuf;

use crate::input::{self, Keybind};
use crate::render::Face;
use crate::theme;

const KEYBIND_KEY: &str = "keybind";

pub const USAGE: &str = "\
Usage: koral [OPTIONS] [-e COMMAND [ARGS...]]
       koral ctl [--socket PATH] COMMAND [ARGS...]
       koral +terminfo install

  -h, --help                   print this help
  --version                    print the version
  --working-directory, --cwd   initial working directory
  --title                      initial window and tab title
  --class, --app-id            window class / Wayland app-id
  -e, --command                run COMMAND with ARGS in the initial tab
  ctl                          send a command to a running koral window
  +terminfo install            install the xterm-koral terminfo entry into ~/.terminfo
";

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Args {
    pub cwd: Option<PathBuf>,
    pub title: Option<String>,
    pub class: Option<String>,
    pub command: Option<(String, Vec<String>)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    Help,
    Version,
    Run(Args),
    Control(Vec<String>),
    TerminfoInstall,
}

pub fn parse_args<I, S>(args: I) -> Result<Launch, String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut iter = args.into_iter().map(Into::into).peekable();
    if iter.peek().is_some_and(|first| first == "ctl") {
        iter.next();
        return Ok(Launch::Control(iter.collect()));
    }
    if iter.peek().is_some_and(|first| first == "+terminfo") {
        iter.next();
        return match iter.next().as_deref() {
            Some("install") if iter.next().is_none() => Ok(Launch::TerminfoInstall),
            Some("install") => Err("unexpected arguments after '+terminfo install'".to_string()),
            Some(other) => Err(format!(
                "unknown +terminfo subcommand '{other}', expected 'install'"
            )),
            None => Err("missing subcommand for '+terminfo', expected 'install'".to_string()),
        };
    }
    let mut parsed = Args::default();
    while let Some(arg) = iter.next() {
        let mut take_value = |flag: &str| {
            iter.next()
                .ok_or_else(|| format!("missing value for '{flag}'"))
        };
        if let Some(value) = arg
            .strip_prefix("--working-directory=")
            .or_else(|| arg.strip_prefix("--cwd="))
        {
            parsed.cwd = Some(PathBuf::from(value));
        } else if let Some(value) = arg.strip_prefix("--title=") {
            parsed.title = Some(value.to_string());
        } else if let Some(value) = arg
            .strip_prefix("--class=")
            .or_else(|| arg.strip_prefix("--app-id="))
        {
            parsed.class = Some(value.to_string());
        } else {
            match arg.as_str() {
                "--version" => return Ok(Launch::Version),
                "-h" | "--help" => return Ok(Launch::Help),
                "--working-directory" | "--cwd" => {
                    parsed.cwd = Some(PathBuf::from(take_value(&arg)?))
                }
                "--title" => parsed.title = Some(take_value(&arg)?),
                "--class" | "--app-id" => parsed.class = Some(take_value(&arg)?),
                "-e" | "--command" | "--" => {
                    let prog = iter
                        .next()
                        .ok_or_else(|| format!("missing command for '{arg}'"))?;
                    parsed.command = Some((prog, iter.collect()));
                    break;
                }
                _ => return Err(format!("unknown argument '{arg}'")),
            }
        }
    }
    Ok(Launch::Run(parsed))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PermissionPolicy {
    /// Let the operation through without asking.
    Allow,
    /// Hold the operation until the user confirms.
    #[default]
    Ask,
    /// Refuse the operation.
    Deny,
}

impl PermissionPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Ask => "ask",
            Self::Deny => "deny",
        }
    }
}

fn policy(value: &str) -> Result<PermissionPolicy, String> {
    match value {
        "allow" => Ok(PermissionPolicy::Allow),
        "ask" => Ok(PermissionPolicy::Ask),
        "deny" => Ok(PermissionPolicy::Deny),
        _ => Err(format!("'{value}' is not allow, ask or deny")),
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum PresentModePolicy {
    #[default]
    Auto,
    Fifo,
    Mailbox,
    Immediate,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub font: String,
    pub font_size: f32,
    pub opacity: f32,
    pub padding: f32,
    pub shell: Option<String>,
    pub shell_integration: bool,
    pub matugen: bool,
    pub window_decorations: bool,
    pub present_mode: PresentModePolicy,
    pub theme: Option<String>,
    pub scrollback: usize,
    pub clipboard_read: bool,
    pub clipboard_write: PermissionPolicy,
    pub notifications: PermissionPolicy,
    pub bindings: Vec<Keybind>,
    pub font_weight: u16,
    pub bold_weight: u16,
    pub bold_italic_weight: u16,
    pub oblique: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            font: "IosevkaTerm Nerd Font Mono".to_string(),
            font_size: 13.5,
            opacity: 0.8,
            padding: 8.0,
            shell: None,
            shell_integration: true,
            matugen: false,
            window_decorations: false,
            present_mode: PresentModePolicy::Auto,
            theme: None,
            scrollback: 10_000,
            clipboard_read: false,
            clipboard_write: PermissionPolicy::Ask,
            notifications: PermissionPolicy::Allow,
            bindings: Vec::new(),
            font_weight: 500,
            bold_weight: 800,
            bold_italic_weight: 900,
            oblique: true,
        }
    }
}

impl Config {
    pub fn face(&self) -> Face {
        Face {
            family: self.font.clone(),
            weight: self.font_weight,
            bold: self.bold_weight,
            bold_italic: self.bold_italic_weight,
            oblique: self.oblique,
        }
    }
}

pub fn home() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
}

pub fn path() -> Option<PathBuf> {
    Some(home()?.join("koral/koral.conf"))
}

pub fn log_path() -> Option<PathBuf> {
    Some(home()?.join("koral/koral.log"))
}

/// Best effort: the same line also reaches the log, which survives a launch from the desktop.
pub fn log(message: &str) {
    if let Some(path) = log_path() {
        let _ = append(&path, message);
    }
}

/// One timestamped line per event, appended.
pub fn append(path: &std::path::Path, message: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "[{seconds}] {message}")
}

/// Legacy tokenizer matching the old `sharky-config` parser for one-time migration and `theme.sharky`.
pub(crate) fn tokenize_legacy(line: &str) -> Result<Vec<String>, String> {
    let chars: Vec<char> = line.chars().collect();
    let mut tokens = Vec::new();
    let mut pos = 0;

    while pos < chars.len() {
        match chars[pos] {
            ' ' | '\t' | '\r' => pos += 1,
            '/' if chars.get(pos + 1) == Some(&'/') => break,
            '"' => {
                let (value, next) = quoted_legacy(&chars, pos)?;
                tokens.push(value);
                pos = next;
            }
            _ => {
                let start = pos;
                while pos < chars.len() {
                    if chars[pos].is_whitespace() {
                        break;
                    }
                    if chars[pos] == '/' && chars.get(pos + 1) == Some(&'/') {
                        break;
                    }
                    pos += 1;
                }
                tokens.push(chars[start..pos].iter().collect());
            }
        }
    }

    Ok(tokens)
}

fn quoted_legacy(chars: &[char], start: usize) -> Result<(String, usize), String> {
    let mut value = String::new();
    let mut pos = start + 1;

    while pos < chars.len() {
        match chars[pos] {
            '"' => return Ok((value, pos + 1)),
            '\\' => {
                let Some(&next) = chars.get(pos + 1) else {
                    return Err("trailing backslash".to_string());
                };
                value.push(match next {
                    'n' => '\n',
                    't' => '\t',
                    other => other,
                });
                pos += 2;
            }
            ch => {
                value.push(ch);
                pos += 1;
            }
        }
    }

    Err("unclosed quote".to_string())
}

fn legacy_comment_start(line: &str) -> Option<usize> {
    let tokens = tokenize_legacy(line).ok()?;
    line.match_indices("//")
        .map(|(index, _)| index)
        .find(|index| tokenize_legacy(&line[..*index]).is_ok_and(|words| words == tokens))
}

/// Converts a legacy `config.sharky` (or `config.koral`) once into `koral.conf` without altering the source file.
pub fn migrate(dir: &std::path::Path) -> std::io::Result<Option<PathBuf>> {
    let target = dir.join("koral.conf");
    if target.exists() {
        return Ok(None);
    }
    let old = [dir.join("config.sharky"), dir.join("config.koral")]
        .into_iter()
        .find(|candidate| candidate.exists());
    let Some(old) = old else {
        return Ok(None);
    };

    let source = std::fs::read_to_string(&old)?;
    let mut out = Vec::new();
    for line in source.lines() {
        let raw = line.strip_suffix('\r').unwrap_or(line);
        let trimmed = raw.trim_start();
        if trimmed.is_empty() {
            out.push(String::new());
            continue;
        }
        if let Some(comment) = trimmed.strip_prefix("//") {
            out.push(format!("#{comment}"));
            continue;
        }
        let Ok(tokens) = tokenize_legacy(raw) else {
            log(&format!("koral.conf migration skipped: {raw}"));
            out.push(format!("# not migrated: {raw}"));
            continue;
        };
        if let Some(start) = legacy_comment_start(raw)
            && let Some(comment) = raw[start..].strip_prefix("//")
        {
            out.push(format!("#{comment}"));
        }
        match tokens.as_slice() {
            [instr, key, value] if instr == "polyp" => {
                out.push(format!("{key} {}", quote(value)));
            }
            [instr, shortcut, action] if instr == "bind" => {
                out.push(format!("{KEYBIND_KEY} {shortcut}={action}"));
            }
            _ => {
                log(&format!("koral.conf migration skipped: {raw}"));
                out.push(format!("# not migrated: {raw}"));
            }
        }
    }

    let mut converted = out.join("\n");
    if !source.is_empty() {
        converted.push('\n');
    }
    save(&target, &converted)?;
    log(&format!("migrated {} to koral.conf", old.display()));
    Ok(Some(old))
}

/// Replaces the file in one step, so a crash mid-write cannot lose the config.
pub fn save(path: &std::path::Path, source: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("conf.tmp");
    std::fs::write(&temporary, source)?;
    std::fs::rename(&temporary, path)
}

/// Bad statements are reported and skipped: a typo must not stop the terminal.
pub fn parse(source: &str) -> (Config, Vec<String>) {
    let mut config = Config::default();
    let mut errors = Vec::new();
    for (index, line) in source.lines().enumerate() {
        if let Err(message) = apply(&mut config, line) {
            errors.push(format!("line {}: {message}", index + 1));
        }
    }
    (config, errors)
}

fn unquote_value(raw: &str) -> Result<String, String> {
    if !raw.starts_with('"') {
        return Ok(raw.to_string());
    }
    let chars: Vec<char> = raw.chars().collect();
    let (value, next) = quoted_legacy(&chars, 0)?;
    if chars[next..].iter().any(|ch| !ch.is_whitespace()) {
        return Err("unexpected trailing characters after quote".to_string());
    }
    Ok(value)
}

fn apply(config: &mut Config, line: &str) -> Result<(), String> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return Ok(());
    }
    let Some((key, rest)) = trimmed.split_once(char::is_whitespace) else {
        return Err(format!("'{trimmed}' takes a key and a value"));
    };
    let raw_value = rest.trim();
    if raw_value.is_empty() {
        return Err(format!("'{key}' takes a key and a value"));
    }
    let value = unquote_value(raw_value)?;
    if key == KEYBIND_KEY {
        let Some((shortcut, action)) = value.rsplit_once('=') else {
            return Err("keybind takes SHORTCUT=ACTION".to_string());
        };
        let (mods, bind_key) = input::parse_shortcut(shortcut.trim())?;
        let action = input::parse_action(action.trim())?;
        config.bindings.push(Keybind {
            mods,
            key: bind_key,
            action,
        });
        return Ok(());
    }

    match key {
        "font" if value.is_empty() => Err("font cannot be empty".to_string()),
        "font" => {
            config.font = value;
            Ok(())
        }
        "font_weight" => weight(&value).map(|weight| config.font_weight = weight),
        "bold_weight" => weight(&value).map(|weight| config.bold_weight = weight),
        "bold_italic_weight" => weight(&value).map(|weight| config.bold_italic_weight = weight),
        "italic_style" => match value.as_str() {
            "oblique" | "italic" => {
                config.oblique = value == "oblique";
                Ok(())
            }
            _ => Err(format!("'{value}' is not oblique or italic")),
        },
        "font_size" => number(&value, 6.0, 72.0).map(|size| config.font_size = size),
        "opacity" => number(&value, 0.1, 1.0).map(|opacity| config.opacity = opacity),
        "padding" => number(&value, 0.0, 64.0).map(|padding| config.padding = padding),
        "shell" => {
            config.shell = Some(value).filter(|shell| !shell.is_empty());
            Ok(())
        }
        "shell_integration" => toggle(&value).map(|on| config.shell_integration = on),
        "theme" => {
            if value.is_empty() {
                config.theme = None;
                Ok(())
            } else if theme::is_builtin(&value) {
                config.theme = Some(value);
                Ok(())
            } else {
                Err(format!("unknown theme '{value}'"))
            }
        }
        "scrollback" => match value.parse::<usize>() {
            Ok(lines) if lines <= 1_000_000 => {
                config.scrollback = lines;
                Ok(())
            }
            _ => Err(format!("'{value}' is not an integer from 0 to 1000000")),
        },
        "clipboard_read" => toggle(&value).map(|on| config.clipboard_read = on),
        "clipboard_write" => policy(&value).map(|policy| config.clipboard_write = policy),
        "notifications" => policy(&value).map(|policy| config.notifications = policy),
        "matugen" => toggle(&value).map(|on| config.matugen = on),
        "window_decorations" => toggle(&value).map(|on| config.window_decorations = on),
        "present_mode" => {
            config.present_mode = match value.as_str() {
                "auto" => PresentModePolicy::Auto,
                "fifo" => PresentModePolicy::Fifo,
                "mailbox" => PresentModePolicy::Mailbox,
                "immediate" => PresentModePolicy::Immediate,
                _ => return Err(format!("'{value}' is not auto, fifo, mailbox or immediate")),
            };
            Ok(())
        }
        other => Err(format!("unknown setting '{other}'")),
    }
}

fn toggle(value: &str) -> Result<bool, String> {
    match value {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(format!("'{value}' is not on or off")),
    }
}

fn weight(value: &str) -> Result<u16, String> {
    number(value, 100.0, 900.0).map(|weight| weight as u16)
}

fn number(value: &str, min: f32, max: f32) -> Result<f32, String> {
    match value.parse::<f32>() {
        Ok(number) if (min..=max).contains(&number) => Ok(number),
        _ => Err(format!("'{value}' is not a number from {min} to {max}")),
    }
}

/// The new file text with one setting changed; every other byte stays.
pub fn set(source: &str, key: &str, value: &str) -> String {
    let statement = format!("{key} {}", quote(value));
    let mut lines: Vec<String> = source.lines().map(str::to_string).collect();

    let found = lines.iter().rposition(|line| {
        let trimmed = line.trim();
        !trimmed.is_empty()
            && !trimmed.starts_with('#')
            && trimmed.split_whitespace().next() == Some(key)
    });
    match found {
        Some(index) => lines[index] = statement,
        None => lines.push(statement),
    }

    let mut result = lines.join("\n");
    result.push('\n');
    result
}

fn quote(value: &str) -> String {
    let needs_quotes = value.is_empty()
        || value.starts_with('"')
        || value.starts_with('#')
        || value.chars().next().is_some_and(char::is_whitespace)
        || value.chars().last().is_some_and(char::is_whitespace)
        || value.contains(['\n', '\t', '\r']);
    if !needs_quotes {
        return value.to_string();
    }
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\t', "\\t");
    format!("\"{escaped}\"")
}

/// `set`, refused when the parser would not take the new value.
pub fn update(source: &str, key: &str, value: &str) -> Result<(String, Config), String> {
    let updated = set(source, key, value);
    let (config, errors) = parse(&updated);
    let before = parse(source).1.len();
    match errors.into_iter().nth(before) {
        Some(message) => Err(message),
        None => Ok((updated, config)),
    }
}

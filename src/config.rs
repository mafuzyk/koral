use std::path::PathBuf;

const INSTRUCTION: &str = "polyp";

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub font: String,
    pub font_size: f32,
    pub opacity: f32,
    pub padding: f32,
    pub shell: Option<String>,
    pub matugen: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            font: "IosevkaTerm Nerd Font Mono".to_string(),
            font_size: 13.5,
            opacity: 0.8,
            padding: 8.0,
            shell: None,
            matugen: false,
        }
    }
}

pub fn home() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
}

pub fn path() -> Option<PathBuf> {
    Some(home()?.join("koral/config.koral"))
}

/// Replaces the file in one step, so a crash mid-write cannot lose the config.
pub fn save(path: &std::path::Path, source: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("koral.tmp");
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

fn apply(config: &mut Config, line: &str) -> Result<(), String> {
    let tokens = tokenize(line)?.0;
    let Some((instruction, rest)) = tokens.split_first() else {
        return Ok(());
    };
    if instruction != INSTRUCTION {
        return Err(format!("unknown instruction '{instruction}', expected {INSTRUCTION}"));
    }
    let [key, value] = rest else {
        return Err(format!("{INSTRUCTION} takes a key and a value"));
    };

    match key.as_str() {
        "font" if value.is_empty() => Err("font cannot be empty".to_string()),
        "font" => {
            config.font = value.clone();
            Ok(())
        }
        "font_size" => number(value, 6.0, 72.0).map(|size| config.font_size = size),
        "opacity" => number(value, 0.1, 1.0).map(|opacity| config.opacity = opacity),
        "padding" => number(value, 0.0, 64.0).map(|padding| config.padding = padding),
        "shell" => {
            config.shell = Some(value.clone()).filter(|shell| !shell.is_empty());
            Ok(())
        }
        "matugen" => match value.as_str() {
            "on" | "off" => {
                config.matugen = value == "on";
                Ok(())
            }
            _ => Err(format!("'{value}' is not on or off")),
        },
        other => Err(format!("unknown setting '{other}'")),
    }
}

fn number(value: &str, min: f32, max: f32) -> Result<f32, String> {
    match value.parse::<f32>() {
        Ok(number) if (min..=max).contains(&number) => Ok(number),
        _ => Err(format!("'{value}' is not a number from {min} to {max}")),
    }
}

/// Words of a line, and where its trailing comment starts.
// shortcut: repeats sharky's tokenizer, swap for sharky-config once it is tagged.
fn tokenize(line: &str) -> Result<(Vec<String>, Option<usize>), String> {
    let mut tokens = Vec::new();
    let mut chars = line.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        if ch.is_whitespace() {
            continue;
        }
        if line[index..].starts_with("//") {
            return Ok((tokens, Some(index)));
        }
        let mut token = String::new();
        if ch == '"' {
            loop {
                match chars.next() {
                    None => return Err("unterminated quote".to_string()),
                    Some((_, '"')) => break,
                    Some((_, '\\')) => match chars.next() {
                        Some((_, 'n')) => token.push('\n'),
                        Some((_, 't')) => token.push('\t'),
                        Some((_, other)) => token.push(other),
                        None => return Err("unterminated quote".to_string()),
                    },
                    Some((_, other)) => token.push(other),
                }
            }
        } else {
            token.push(ch);
            while let Some((_, next)) = chars.peek().copied() {
                if next.is_whitespace() {
                    break;
                }
                token.push(next);
                chars.next();
            }
        }
        tokens.push(token);
    }
    Ok((tokens, None))
}

/// The new file text with one setting changed; every other byte stays.
pub fn set(source: &str, key: &str, value: &str) -> String {
    let statement = format!("{INSTRUCTION} {key} {}", quote(value));
    let mut lines: Vec<String> = source.lines().map(str::to_string).collect();

    let found = lines.iter().rposition(|line| {
        tokenize(line).is_ok_and(|(tokens, _)| {
            tokens.first().is_some_and(|word| word == INSTRUCTION) && tokens.get(1).is_some_and(|word| word == key)
        })
    });
    match found {
        Some(index) => {
            let comment = tokenize(&lines[index]).ok().and_then(|(_, start)| start);
            lines[index] = match comment {
                Some(start) => format!("{statement} {}", &lines[index][start..]),
                None => statement,
            };
        }
        None => lines.push(statement),
    }

    let mut result = lines.join("\n");
    result.push('\n');
    result
}

fn quote(value: &str) -> String {
    let plain = !value.is_empty()
        && !value.contains("//")
        && value.chars().all(|ch| !ch.is_whitespace() && ch != '"' && ch != '\\');
    if plain {
        return value.to_string();
    }
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\t', "\\t");
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

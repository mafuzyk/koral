use std::collections::HashMap;

use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, Rgb};

const fn rgb(value: u32) -> Rgb {
    Rgb {
        r: (value >> 16) as u8,
        g: (value >> 8) as u8,
        b: value as u8,
    }
}

/// A `#RRGGBB` colour, the only form `theme.sharky` allows.
pub fn hex(text: &str) -> Option<Rgb> {
    let digits = text.strip_prefix('#')?;
    if digits.len() != 6 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some(rgb(u32::from_str_radix(digits, 16).ok()?))
}

const SHARKY_NAMES: [(&str, &str); 6] = [
    ("abyss", "background"),
    ("foam", "foreground"),
    ("sea", "accent"),
    ("sea", "cursor"),
    ("kelp", "success"),
    ("danger", "failure"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    pub background: Rgb,
    pub foreground: Rgb,
    pub cursor: Rgb,
    pub accent: Rgb,
    pub success: Rgb,
    pub failure: Rgb,
    pub muted: Rgb,
    pub ansi: [Rgb; 16],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeInfo {
    pub id: &'static str,
    pub name: &'static str,
}

pub const BUILTINS: &[ThemeInfo] = &[
    ThemeInfo {
        id: "koral",
        name: "Koral",
    },
    ThemeInfo {
        id: "sharky",
        name: "Sharky (theme.sharky)",
    },
    ThemeInfo {
        id: "terminal",
        name: "Terminal (Classic)",
    },
    ThemeInfo {
        id: "dracula",
        name: "Dracula",
    },
    ThemeInfo {
        id: "catppuccin-mocha",
        name: "Catppuccin Mocha",
    },
    ThemeInfo {
        id: "catppuccin-macchiato",
        name: "Catppuccin Macchiato",
    },
    ThemeInfo {
        id: "catppuccin-frappe",
        name: "Catppuccin Frappé",
    },
    ThemeInfo {
        id: "catppuccin-latte",
        name: "Catppuccin Latte",
    },
    ThemeInfo {
        id: "tokyo-night",
        name: "Tokyo Night",
    },
    ThemeInfo {
        id: "gruvbox-dark",
        name: "Gruvbox Dark",
    },
    ThemeInfo {
        id: "nord",
        name: "Nord",
    },
    ThemeInfo {
        id: "rose-pine",
        name: "Rosé Pine",
    },
    ThemeInfo {
        id: "one-dark",
        name: "One Dark",
    },
    ThemeInfo {
        id: "solarized-dark",
        name: "Solarized Dark",
    },
    ThemeInfo {
        id: "high-contrast",
        name: "High Contrast",
    },
];

const fn preset(ui: [u32; 7], ansi: [u32; 16]) -> Theme {
    Theme {
        background: rgb(ui[0]),
        foreground: rgb(ui[1]),
        cursor: rgb(ui[2]),
        accent: rgb(ui[3]),
        success: rgb(ui[4]),
        failure: rgb(ui[5]),
        muted: rgb(ui[6]),
        ansi: [
            rgb(ansi[0]),
            rgb(ansi[1]),
            rgb(ansi[2]),
            rgb(ansi[3]),
            rgb(ansi[4]),
            rgb(ansi[5]),
            rgb(ansi[6]),
            rgb(ansi[7]),
            rgb(ansi[8]),
            rgb(ansi[9]),
            rgb(ansi[10]),
            rgb(ansi[11]),
            rgb(ansi[12]),
            rgb(ansi[13]),
            rgb(ansi[14]),
            rgb(ansi[15]),
        ],
    }
}

pub fn is_builtin(id: &str) -> bool {
    BUILTINS.iter().any(|info| info.id == id)
}

pub fn theme_name(id: &str) -> Option<&'static str> {
    BUILTINS
        .iter()
        .find(|info| info.id == id)
        .map(|info| info.name)
}

/// Blends `a` toward `b` by `t` (`0.0..=1.0`), safe for both dark and light themes.
pub fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    let ch = |x: u8, y: u8| (f32::from(x) * (1.0 - t) + f32::from(y) * t).round() as u8;
    Rgb {
        r: ch(a.r, b.r),
        g: ch(a.g, b.g),
        b: ch(a.b, b.b),
    }
}

// The colours of the example `theme.sharky`, for a machine without one.
impl Default for Theme {
    fn default() -> Self {
        Self {
            background: rgb(0x071923),
            foreground: rgb(0xE4F5F4),
            cursor: rgb(0x35B8C5),
            accent: rgb(0x35B8C5),
            success: rgb(0x8CCFA8),
            failure: rgb(0xFF8F80),
            muted: rgb(0x7E9BA6),
            ansi: [
                rgb(0x0E2A38),
                rgb(0xFF8F80),
                rgb(0x8CCFA8),
                rgb(0xE9C98A),
                rgb(0x6FA8DC),
                rgb(0xC8A2E0),
                rgb(0x35B8C5),
                rgb(0xC5DAD9),
                rgb(0x4A6A78),
                rgb(0xFFB3A8),
                rgb(0xB0E4C6),
                rgb(0xF4DDB0),
                rgb(0x9CC6EC),
                rgb(0xDCC3EC),
                rgb(0x7FD6DF),
                rgb(0xF4FBFA),
            ],
        }
    }
}

impl Theme {
    /// The built-in colours with the named ones replaced: a damaged line changes nothing.
    pub fn with_colors(source: &str) -> Self {
        let mut theme = Self::default();
        for line in source.lines() {
            let mut words = line.split_whitespace();
            let (Some(name), Some(color)) = (words.next(), words.next().and_then(hex)) else {
                continue;
            };
            let slot = theme.slot(name);
            if let Some(slot) = slot {
                *slot = color;
            }
        }
        theme
    }

    fn slot(&mut self, name: &str) -> Option<&mut Rgb> {
        match name {
            "background" => Some(&mut self.background),
            "foreground" => Some(&mut self.foreground),
            "cursor" => Some(&mut self.cursor),
            "accent" => Some(&mut self.accent),
            "success" => Some(&mut self.success),
            "failure" => Some(&mut self.failure),
            "muted" => Some(&mut self.muted),
            _ => name
                .strip_prefix("ansi")
                .and_then(|index| index.parse::<usize>().ok())
                .and_then(|index| self.ansi.get_mut(index)),
        }
    }

    /// Sharky's own colour names first, so a theme that never mentions the terminal still applies.
    pub fn with_sharky(source: &str, file: &str) -> Result<Self, String> {
        let parsed = parse_sharky_theme(source, file)?;
        let mut theme = Self::default();
        for (name, slot) in SHARKY_NAMES {
            if let (Some(color), Some(slot)) = (
                parsed.colors.get(name).and_then(|text| hex(text)),
                theme.slot(slot),
            ) {
                *slot = color;
            }
        }
        for (key, value) in &parsed.styles {
            let color = match value.strip_prefix('@') {
                Some(name) => parsed.colors.get(name).map(String::as_str),
                None => Some(value.as_str()),
            };
            let slot = key
                .strip_prefix("terminal.")
                .and_then(|name| theme.slot(name));
            if let (Some(color), Some(slot)) = (color.and_then(hex), slot) {
                *slot = color;
            }
        }
        Ok(theme)
    }

    /// The user's `theme.sharky`. A bad theme is reported and must not stop the terminal.
    pub fn sharky() -> Self {
        let path = crate::config::home().map(|home| home.join("sharky/theme.sharky"));
        let source = path
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok());
        let Some((path, source)) = path.zip(source) else {
            return Self::default();
        };
        Self::with_sharky(&source, &path.display().to_string()).unwrap_or_else(|message| {
            eprintln!("koral: {message}");
            Self::default()
        })
    }

    pub fn builtin(id: &str) -> Option<Self> {
        let theme = match id {
            "koral" => Self::default(),
            "sharky" => Self::sharky(),
            "terminal" => preset(
                [
                    0x101010, 0xE5E5E5, 0xE5E5E5, 0x3B8EEA, 0x23D18B, 0xF14C4C, 0x767676,
                ],
                [
                    0x000000, 0xCD3131, 0x0DBC79, 0xE5E510, 0x2472C8, 0xBC3FBC, 0x11A8CD, 0xE5E5E5,
                    0x666666, 0xF14C4C, 0x23D18B, 0xF5F543, 0x3B8EEA, 0xD670D6, 0x29B8DB, 0xFFFFFF,
                ],
            ),
            "dracula" => preset(
                [
                    0x282A36, 0xF8F8F2, 0xF8F8F2, 0xBD93F9, 0x50FA7B, 0xFF5555, 0x6272A4,
                ],
                [
                    0x21222C, 0xFF5555, 0x50FA7B, 0xF1FA8C, 0xBD93F9, 0xFF79C6, 0x8BE9FD, 0xF8F8F2,
                    0x6272A4, 0xFF6E6E, 0x69FF94, 0xFFFFA5, 0xD6ACFF, 0xFF92DF, 0xA4FFFF, 0xFFFFFF,
                ],
            ),
            "catppuccin-mocha" => preset(
                [
                    0x1E1E2E, 0xCDD6F4, 0xF5E0DC, 0xCBA6F7, 0xA6E3A1, 0xF38BA8, 0x6C7086,
                ],
                [
                    0x45475A, 0xF38BA8, 0xA6E3A1, 0xF9E2AF, 0x89B4FA, 0xF5C2E7, 0x94E2D5, 0xBAC2DE,
                    0x585B70, 0xF38BA8, 0xA6E3A1, 0xF9E2AF, 0x89B4FA, 0xF5C2E7, 0x94E2D5, 0xA6ADC8,
                ],
            ),
            "catppuccin-macchiato" => preset(
                [
                    0x24273A, 0xCAD3F5, 0xF4DBD6, 0xC6A0F6, 0xA6DA95, 0xED8796, 0x6E738D,
                ],
                [
                    0x494D64, 0xED8796, 0xA6DA95, 0xEED49F, 0x8AADF4, 0xF5BDE6, 0x8BD5CA, 0xB8C0E0,
                    0x5B6078, 0xED8796, 0xA6DA95, 0xEED49F, 0x8AADF4, 0xF5BDE6, 0x8BD5CA, 0xA5ADCB,
                ],
            ),
            "catppuccin-frappe" => preset(
                [
                    0x303446, 0xC6D0F5, 0xF2D5CF, 0xCA9EE6, 0xA6D189, 0xE78284, 0x737994,
                ],
                [
                    0x51576D, 0xE78284, 0xA6D189, 0xE5C890, 0x8CAAEE, 0xF4B8E4, 0x81C8BE, 0xB5BFE2,
                    0x626880, 0xE78284, 0xA6D189, 0xE5C890, 0x8CAAEE, 0xF4B8E4, 0x81C8BE, 0xA5ADCE,
                ],
            ),
            "catppuccin-latte" => preset(
                [
                    0xEFF1F5, 0x4C4F69, 0xDC8A78, 0x1E66F5, 0x40A02B, 0xD20F39, 0x6C6F85,
                ],
                [
                    0x5C5F77, 0xD20F39, 0x40A02B, 0xDF8E1D, 0x1E66F5, 0xEA76CB, 0x179299, 0xACB0BE,
                    0x6C6F85, 0xDE293E, 0x49AF31, 0xEEA02D, 0x456EFF, 0xFE85D8, 0x2D9FA8, 0xBCC0CC,
                ],
            ),
            "tokyo-night" => preset(
                [
                    0x1A1B26, 0xC0CAF5, 0xC0CAF5, 0x7AA2F7, 0x9ECE6A, 0xF7768E, 0x565F89,
                ],
                [
                    0x15161E, 0xF7768E, 0x9ECE6A, 0xE0AF68, 0x7AA2F7, 0xBB9AF7, 0x7DCFFF, 0xA9B1D6,
                    0x414868, 0xF7768E, 0x9ECE6A, 0xE0AF68, 0x7AA2F7, 0xBB9AF7, 0x7DCFFF, 0xC0CAF5,
                ],
            ),
            "gruvbox-dark" => preset(
                [
                    0x282828, 0xEBDBB2, 0xEBDBB2, 0xFE8019, 0xB8BB26, 0xFB4934, 0x928374,
                ],
                [
                    0x282828, 0xCC241D, 0x98971A, 0xD79921, 0x458588, 0xB16286, 0x689D6A, 0xA89984,
                    0x928374, 0xFB4934, 0xB8BB26, 0xFABD2F, 0x83A598, 0xD3869B, 0x8EC07C, 0xEBDBB2,
                ],
            ),
            "nord" => preset(
                [
                    0x2E3440, 0xD8DEE9, 0xD8DEE9, 0x88C0D0, 0xA3BE8C, 0xBF616A, 0x616E88,
                ],
                [
                    0x3B4252, 0xBF616A, 0xA3BE8C, 0xEBCB8B, 0x81A1C1, 0xB48EAD, 0x88C0D0, 0xE5E9F0,
                    0x4C566A, 0xBF616A, 0xA3BE8C, 0xEBCB8B, 0x81A1C1, 0xB48EAD, 0x8FBCBB, 0xECEFF4,
                ],
            ),
            "rose-pine" => preset(
                [
                    0x191724, 0xE0DEF4, 0x524F67, 0xC4A7E7, 0x9CCFD8, 0xEB6F92, 0x6E6A86,
                ],
                [
                    0x26233A, 0xEB6F92, 0x31748F, 0xF6C177, 0x9CCFD8, 0xC4A7E7, 0xEBBCBA, 0xE0DEF4,
                    0x6E6A86, 0xEB6F92, 0x31748F, 0xF6C177, 0x9CCFD8, 0xC4A7E7, 0xEBBCBA, 0xE0DEF4,
                ],
            ),
            "one-dark" => preset(
                [
                    0x282C34, 0xABB2BF, 0x528BFF, 0x61AFEF, 0x98C379, 0xE06C75, 0x5C6370,
                ],
                [
                    0x1E2127, 0xE06C75, 0x98C379, 0xD19A66, 0x61AFEF, 0xC678DD, 0x56B6C2, 0xABB2BF,
                    0x5C6370, 0xE06C75, 0x98C379, 0xE5C07B, 0x61AFEF, 0xC678DD, 0x56B6C2, 0xFFFFFF,
                ],
            ),
            "solarized-dark" => preset(
                [
                    0x002B36, 0x93A1A1, 0x93A1A1, 0x268BD2, 0x859900, 0xDC322F, 0x657B83,
                ],
                [
                    0x073642, 0xDC322F, 0x859900, 0xB58900, 0x268BD2, 0xD33682, 0x2AA198, 0xEEE8D5,
                    0x002B36, 0xCB4B16, 0x586E75, 0x657B83, 0x839496, 0x6C71C4, 0x93A1A1, 0xFDF6E3,
                ],
            ),
            "high-contrast" => preset(
                [
                    0x000000, 0xFFFFFF, 0x00FFFF, 0x00FFFF, 0x00FF00, 0xFF4444, 0xB8B8B8,
                ],
                [
                    0x000000, 0xFF5555, 0x55FF55, 0xFFFF55, 0x5599FF, 0xFF55FF, 0x55FFFF, 0xFFFFFF,
                    0x808080, 0xFF7777, 0x77FF77, 0xFFFF88, 0x88BBFF, 0xFF88FF, 0x88FFFF, 0xFFFFFF,
                ],
            ),
            _ => return None,
        };
        Some(theme)
    }

    pub fn resolve_named(name: Option<&str>) -> Self {
        match name {
            None | Some("") | Some("sharky") => Self::sharky(),
            Some(id) => Self::builtin(id).unwrap_or_else(Self::sharky),
        }
    }

    pub fn resolve(&self, color: Color, overrides: &Colors) -> Rgb {
        let index = match color {
            Color::Spec(rgb) => return rgb,
            Color::Named(named) => named as usize,
            Color::Indexed(index) => usize::from(index),
        };
        overrides[index].unwrap_or_else(|| self.palette(index))
    }

    pub fn palette(&self, index: usize) -> Rgb {
        match index {
            0..=15 => self.ansi[index],
            16..=231 => {
                let index = index - 16;
                let level = |step: usize| if step == 0 { 0 } else { (55 + step * 40) as u8 };
                Rgb {
                    r: level(index / 36),
                    g: level(index / 6 % 6),
                    b: level(index % 6),
                }
            }
            232..=255 => {
                let level = (8 + (index - 232) * 10) as u8;
                Rgb {
                    r: level,
                    g: level,
                    b: level,
                }
            }
            256 | 267 => self.foreground,
            257 => self.background,
            258 => self.cursor,
            259..=266 => dim(self.ansi[index - 259]),
            _ => dim(self.foreground),
        }
    }
}

fn dim(color: Rgb) -> Rgb {
    let scale = |value: u8| (u16::from(value) * 2 / 3) as u8;
    Rgb {
        r: scale(color.r),
        g: scale(color.g),
        b: scale(color.b),
    }
}

struct SharkyTheme {
    colors: HashMap<String, String>,
    styles: HashMap<String, String>,
}

fn parse_sharky_theme(source: &str, file: &str) -> Result<SharkyTheme, String> {
    let mut colors = HashMap::new();
    let mut styles = HashMap::new();
    for (index, line) in source.lines().enumerate() {
        let number = index + 1;
        let tokens = crate::config::tokenize_legacy(line)
            .map_err(|message| format!("{file}:{number}: {message}"))?;
        let Some((instruction, rest)) = tokens.split_first() else {
            continue;
        };
        match instruction.as_str() {
            "coral" => {
                let [key, value] = rest else {
                    return Err(format!("{file}:{number}: coral takes a name and a colour"));
                };
                if hex(value).is_none() {
                    return Err(format!(
                        "{file}:{number}: '{value}' is not a colour like #071923"
                    ));
                }
                colors.insert(key.clone(), value.clone());
            }
            "wave" => {
                let [key, value] = rest else {
                    return Err(format!("{file}:{number}: wave takes a key and a value"));
                };
                styles.insert(key.clone(), value.clone());
            }
            "tide" | "pearl" | "current" | "reef" | "shoal" | "anchor" | "wake" => {}
            other => {
                return Err(format!(
                    "{file}:{number}: unknown instruction '{other}' in theme.sharky"
                ));
            }
        }
    }
    for (key, value) in &styles {
        if let Some(reference) = value.strip_prefix('@')
            && !colors.contains_key(reference)
        {
            return Err(format!(
                "{file}:0: {key} refers to undefined colour '@{reference}'"
            ));
        }
    }
    Ok(SharkyTheme { colors, styles })
}

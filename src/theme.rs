use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, Rgb};

const fn rgb(value: u32) -> Rgb {
    Rgb { r: (value >> 16) as u8, g: (value >> 8) as u8, b: value as u8 }
}

/// A `#RRGGBB` colour, the only form `theme.sharky` allows.
pub fn hex(text: &str) -> Option<Rgb> {
    let digits = text.strip_prefix('#')?;
    if digits.len() != 6 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some(rgb(u32::from_str_radix(digits, 16).ok()?))
}

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

// The colours of the example `theme.sharky`, until sharky-config can load the real one.
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
                Rgb { r: level(index / 36), g: level(index / 6 % 6), b: level(index % 6) }
            }
            232..=255 => {
                let level = (8 + (index - 232) * 10) as u8;
                Rgb { r: level, g: level, b: level }
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
    Rgb { r: scale(color.r), g: scale(color.g), b: scale(color.b) }
}

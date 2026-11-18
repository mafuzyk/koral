use std::time::{Duration, Instant};

use alacritty_terminal::selection::SelectionType;
use alacritty_terminal::term::TermMode;
use winit::keyboard::{Key, ModifiersState, NamedKey};

pub fn encode(key: &Key, text: Option<&str>, mods: ModifiersState, mode: TermMode) -> Option<Vec<u8>> {
    match key {
        Key::Named(named) => encode_named(*named, text, mods, mode),
        Key::Character(chars) => encode_text(text.unwrap_or(chars), mods),
        _ => text.and_then(|text| encode_text(text, mods)),
    }
}

fn encode_text(text: &str, mods: ModifiersState) -> Option<Vec<u8>> {
    let bytes = if mods.control_key() {
        let mut chars = text.chars();
        let (ch, None) = (chars.next()?, chars.next()) else {
            return None;
        };
        vec![control_code(ch)?]
    } else {
        text.as_bytes().to_vec()
    };
    Some(with_alt(bytes, mods))
}

fn control_code(ch: char) -> Option<u8> {
    match ch.to_ascii_uppercase() {
        ch @ '@'..='_' => Some(ch as u8 - b'@'),
        '?' => Some(0x7f),
        _ => None,
    }
}

fn with_alt(mut bytes: Vec<u8>, mods: ModifiersState) -> Vec<u8> {
    if mods.alt_key() {
        bytes.insert(0, 0x1b);
    }
    bytes
}

fn encode_named(key: NamedKey, text: Option<&str>, mods: ModifiersState, mode: TermMode) -> Option<Vec<u8>> {
    let code = modifier_code(mods);
    let letter = match key {
        NamedKey::ArrowUp => Some('A'),
        NamedKey::ArrowDown => Some('B'),
        NamedKey::ArrowRight => Some('C'),
        NamedKey::ArrowLeft => Some('D'),
        NamedKey::Home => Some('H'),
        NamedKey::End => Some('F'),
        _ => None,
    };
    if let Some(letter) = letter {
        return Some(match (code, mode.contains(TermMode::APP_CURSOR)) {
            (1, true) => format!("\x1bO{letter}"),
            (1, false) => format!("\x1b[{letter}"),
            _ => format!("\x1b[1;{code}{letter}"),
        }
        .into_bytes());
    }

    let number = match key {
        NamedKey::Insert => Some(2),
        NamedKey::Delete => Some(3),
        NamedKey::PageUp => Some(5),
        NamedKey::PageDown => Some(6),
        NamedKey::F5 => Some(15),
        NamedKey::F6 => Some(17),
        NamedKey::F7 => Some(18),
        NamedKey::F8 => Some(19),
        NamedKey::F9 => Some(20),
        NamedKey::F10 => Some(21),
        NamedKey::F11 => Some(23),
        NamedKey::F12 => Some(24),
        _ => None,
    };
    if let Some(number) = number {
        return Some(match code {
            1 => format!("\x1b[{number}~"),
            _ => format!("\x1b[{number};{code}~"),
        }
        .into_bytes());
    }

    let function = match key {
        NamedKey::F1 => Some('P'),
        NamedKey::F2 => Some('Q'),
        NamedKey::F3 => Some('R'),
        NamedKey::F4 => Some('S'),
        _ => None,
    };
    if let Some(letter) = function {
        return Some(match code {
            1 => format!("\x1bO{letter}"),
            _ => format!("\x1b[1;{code}{letter}"),
        }
        .into_bytes());
    }

    match key {
        NamedKey::Enter => Some(with_alt(vec![b'\r'], mods)),
        NamedKey::Escape => Some(with_alt(vec![0x1b], mods)),
        NamedKey::Tab if mods.shift_key() => Some(b"\x1b[Z".to_vec()),
        NamedKey::Tab => Some(with_alt(vec![b'\t'], mods)),
        NamedKey::Backspace if mods.control_key() => Some(with_alt(vec![0x08], mods)),
        NamedKey::Backspace => Some(with_alt(vec![0x7f], mods)),
        NamedKey::Space if mods.control_key() => Some(with_alt(vec![0], mods)),
        NamedKey::Space => Some(with_alt(vec![b' '], mods)),
        _ => text.map(|text| with_alt(text.as_bytes().to_vec(), mods)),
    }
}

fn modifier_code(mods: ModifiersState) -> u8 {
    1 + u8::from(mods.shift_key()) + 2 * u8::from(mods.alt_key()) + 4 * u8::from(mods.control_key())
}

/// Clipboard text is untrusted: without this filter it could close the bracket
/// and run as typed input.
pub fn paste(text: &str, mode: TermMode) -> Vec<u8> {
    if mode.contains(TermMode::BRACKETED_PASTE) {
        let clean: String = text.chars().filter(|ch| !matches!(ch, '\x1b' | '\u{9b}')).collect();
        return format!("\x1b[200~{clean}\x1b[201~").into_bytes();
    }
    text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
}

#[derive(Debug, Clone, Copy)]
pub struct Mouse {
    pub button: u8,
    pub column: usize,
    pub line: usize,
    pub pressed: bool,
    pub mods: ModifiersState,
}

pub fn mouse_report(mouse: Mouse, mode: TermMode) -> Option<Vec<u8>> {
    if !mode.intersects(TermMode::MOUSE_MODE) {
        return None;
    }

    let bits = 4 * u8::from(mouse.mods.shift_key())
        + 8 * u8::from(mouse.mods.alt_key())
        + 16 * u8::from(mouse.mods.control_key());
    let code = mouse.button + bits;

    if mode.contains(TermMode::SGR_MOUSE) {
        let end = if mouse.pressed { 'M' } else { 'm' };
        let (column, line) = (mouse.column + 1, mouse.line + 1);
        return Some(format!("\x1b[<{code};{column};{line}{end}").into_bytes());
    }

    if mouse.column > 222 || mouse.line > 222 {
        return None;
    }
    let code = if mouse.pressed { code } else { 3 + bits };
    Some(vec![0x1b, b'[', b'M', 32 + code, 33 + mouse.column as u8, 33 + mouse.line as u8])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    NewTab,
    CloseTab,
    NextTab,
    PreviousTab,
    SelectTab(usize),
    Copy,
    Paste,
    PreviousPrompt,
    NextPrompt,
    CopyOutput,
    ToggleFold,
    Search,
    PageUp,
    PageDown,
    OpenSettings,
    ZoomIn,
    ZoomOut,
    ZoomReset,
}

const CLICK_WINDOW: Duration = Duration::from_millis(400);

/// Counts clicks on one cell: a double click takes a word, a third a line.
#[derive(Debug, Default)]
pub struct Clicks {
    last: Option<(Instant, (i32, usize), u8)>,
}

impl Clicks {
    pub fn press(&mut self, cell: (i32, usize), now: Instant) -> u8 {
        let count = match self.last {
            Some((at, seen, count)) if seen == cell && now.duration_since(at) < CLICK_WINDOW => count % 3 + 1,
            _ => 1,
        };
        self.last = Some((now, cell, count));
        count
    }
}

pub fn selection_kind(clicks: u8, mods: ModifiersState) -> SelectionType {
    if mods.control_key() && mods.alt_key() {
        return SelectionType::Block;
    }
    match clicks {
        2 => SelectionType::Semantic,
        3 => SelectionType::Lines,
        _ => SelectionType::Simple,
    }
}

pub fn zoomed(size: f32, delta: f32) -> f32 {
    (size + delta).clamp(6.0, 72.0)
}

pub fn binding(key: &Key, mods: ModifiersState) -> Option<Action> {
    let shift = mods.shift_key();
    match key {
        Key::Named(NamedKey::Tab) if mods.control_key() => {
            Some(if shift { Action::PreviousTab } else { Action::NextTab })
        }
        Key::Named(NamedKey::PageUp) if shift => Some(Action::PageUp),
        Key::Named(NamedKey::PageDown) if shift => Some(Action::PageDown),
        Key::Character(text) if mods == ModifiersState::ALT => match text.parse::<usize>() {
            Ok(digit @ 1..=9) => Some(Action::SelectTab(digit - 1)),
            _ => None,
        },
        Key::Character(text) if mods == ModifiersState::CONTROL => match text.as_str() {
            "=" | "+" => Some(Action::ZoomIn),
            "-" => Some(Action::ZoomOut),
            _ => None,
        },
        Key::Character(text) if mods.control_key() && shift && !mods.alt_key() => {
            match text.to_ascii_lowercase().as_str() {
                "t" => Some(Action::NewTab),
                "w" => Some(Action::CloseTab),
                "c" => Some(Action::Copy),
                "v" => Some(Action::Paste),
                "z" => Some(Action::PreviousPrompt),
                "x" => Some(Action::NextPrompt),
                "g" => Some(Action::CopyOutput),
                "f" => Some(Action::ToggleFold),
                "s" => Some(Action::Search),
                "," | "<" => Some(Action::OpenSettings),
                "=" | "+" => Some(Action::ZoomIn),
                "-" | "_" => Some(Action::ZoomOut),
                "0" | ")" => Some(Action::ZoomReset),
                _ => None,
            }
        }
        _ => None,
    }
}

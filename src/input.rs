use std::time::{Duration, Instant};

use alacritty_terminal::selection::SelectionType;
use alacritty_terminal::term::TermMode;
use winit::keyboard::{Key, ModifiersState, NamedKey};

pub fn encode(
    key: &Key,
    text: Option<&str>,
    mods: ModifiersState,
    mode: TermMode,
) -> Option<Vec<u8>> {
    match key {
        Key::Named(named) => encode_named(*named, text, mods, mode),
        Key::Character(chars) => encode_text(text.unwrap_or(chars), mods),
        _ => text.and_then(|text| encode_text(text, mods)),
    }
}

fn encode_text(text: &str, mods: ModifiersState) -> Option<Vec<u8>> {
    if mods.control_key() {
        let mut chars = text.chars();
        if let (Some(ch), None) = (chars.next(), chars.next())
            && let Some(code) = control_code(ch)
        {
            return Some(with_alt(vec![code], mods));
        }
        // AltGr arrives as Ctrl+Alt on Linux and composes a character instead of
        // pressing a control key: no escape prefix, or the app sees Alt+char.
        return (!text.is_ascii()).then(|| text.as_bytes().to_vec());
    }
    Some(with_alt(text.as_bytes().to_vec(), mods))
}

fn control_code(ch: char) -> Option<u8> {
    match ch.to_ascii_uppercase() {
        ch @ '@'..='_' => Some(ch as u8 - b'@'),
        '?' => Some(0x7f),
        '2' => Some(0x00),
        '3' => Some(0x1b),
        '4' => Some(0x1c),
        '5' => Some(0x1d),
        '6' => Some(0x1e),
        '7' => Some(0x1f),
        '8' => Some(0x7f),
        '/' => Some(0x1f),
        '`' | '~' => Some(0x00),
        _ => None,
    }
}

fn with_alt(mut bytes: Vec<u8>, mods: ModifiersState) -> Vec<u8> {
    if mods.alt_key() {
        bytes.insert(0, 0x1b);
    }
    bytes
}

fn encode_named(
    key: NamedKey,
    text: Option<&str>,
    mods: ModifiersState,
    mode: TermMode,
) -> Option<Vec<u8>> {
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
        return Some(
            match (code, mode.contains(TermMode::APP_CURSOR)) {
                (1, true) => format!("\x1bO{letter}"),
                (1, false) => format!("\x1b[{letter}"),
                _ => format!("\x1b[1;{code}{letter}"),
            }
            .into_bytes(),
        );
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
        return Some(
            match code {
                1 => format!("\x1b[{number}~"),
                _ => format!("\x1b[{number};{code}~"),
            }
            .into_bytes(),
        );
    }

    let function = match key {
        NamedKey::F1 => Some('P'),
        NamedKey::F2 => Some('Q'),
        NamedKey::F3 => Some('R'),
        NamedKey::F4 => Some('S'),
        _ => None,
    };
    if let Some(letter) = function {
        return Some(
            match code {
                1 => format!("\x1bO{letter}"),
                _ => format!("\x1b[1;{code}{letter}"),
            }
            .into_bytes(),
        );
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
    let clean: String = text
        .chars()
        .filter(|ch| !matches!(ch, '\x1b' | '\u{9b}'))
        .collect();
    if mode.contains(TermMode::BRACKETED_PASTE) {
        return format!("\x1b[200~{clean}\x1b[201~").into_bytes();
    }
    clean.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
}

#[derive(Debug, Clone, Copy)]
pub struct Mouse {
    pub button: u8,
    pub column: usize,
    pub line: usize,
    pub pressed: bool,
    pub mods: ModifiersState,
    /// A drag or motion event, which xterm encodes with bit 32 set.
    pub motion: bool,
}

/// The button a motion event reports, or `None` when the mode asks for no motion.
/// A grabbed program that never sees motion cannot run its own selection.
pub fn motion_button(mode: TermMode, held: Option<u8>) -> Option<u8> {
    if mode.contains(TermMode::MOUSE_DRAG)
        && let Some(button) = held
    {
        return Some(button);
    }
    mode.contains(TermMode::MOUSE_MOTION)
        .then(|| held.unwrap_or(3))
}

pub fn mouse_report(mouse: Mouse, mode: TermMode) -> Option<Vec<u8>> {
    if !mode.intersects(TermMode::MOUSE_MODE) {
        return None;
    }

    let bits = 4 * u8::from(mouse.mods.shift_key())
        + 8 * u8::from(mouse.mods.alt_key())
        + 16 * u8::from(mouse.mods.control_key());
    let code = mouse.button + bits + if mouse.motion { 32 } else { 0 };

    if mode.contains(TermMode::SGR_MOUSE) {
        let end = if mouse.pressed { 'M' } else { 'm' };
        let (column, line) = (mouse.column + 1, mouse.line + 1);
        return Some(format!("\x1b[<{code};{column};{line}{end}").into_bytes());
    }

    if mouse.column > 222 || mouse.line > 222 {
        return None;
    }
    let code = if mouse.pressed { code } else { 3 + bits };
    Some(vec![
        0x1b,
        b'[',
        b'M',
        32 + code,
        33 + mouse.column as u8,
        33 + mouse.line as u8,
    ])
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
    SplitRight,
    SplitDown,
    ClosePane,
    FocusLeft,
    FocusRight,
    FocusUp,
    FocusDown,
    ResizeLeft,
    ResizeRight,
    ResizeUp,
    ResizeDown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindKey {
    Char(char),
    Named(NamedKey),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keybind {
    pub mods: ModifiersState,
    pub key: BindKey,
    pub action: Option<Action>,
}

pub fn parse_shortcut(spec: &str) -> Result<(ModifiersState, BindKey), String> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return Err("empty keybinding".to_string());
    }
    let (mod_part, key_part) = if trimmed == "+" {
        ("", "+")
    } else if let Some(prefix) = trimmed.strip_suffix("++") {
        (prefix, "+")
    } else if let Some((left, right)) = trimmed.rsplit_once('+') {
        (left, right)
    } else {
        ("", trimmed)
    };
    if key_part.is_empty() {
        return Err(format!("missing key in '{spec}'"));
    }
    let mut mods = ModifiersState::empty();
    if !mod_part.is_empty() {
        for part in mod_part.split('+') {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => mods |= ModifiersState::CONTROL,
                "shift" => mods |= ModifiersState::SHIFT,
                "alt" | "meta" => mods |= ModifiersState::ALT,
                "super" | "logo" | "win" => mods |= ModifiersState::SUPER,
                other => return Err(format!("unknown modifier '{other}' in '{spec}'")),
            }
        }
    }
    let key =
        parse_bind_key(key_part).ok_or_else(|| format!("unknown key '{key_part}' in '{spec}'"))?;
    Ok((mods, key))
}

fn parse_bind_key(part: &str) -> Option<BindKey> {
    let lower = part.to_ascii_lowercase();
    let named = match lower.as_str() {
        "tab" => Some(NamedKey::Tab),
        "enter" | "return" => Some(NamedKey::Enter),
        "esc" | "escape" => Some(NamedKey::Escape),
        "backspace" => Some(NamedKey::Backspace),
        "space" => Some(NamedKey::Space),
        "pageup" | "page_up" => Some(NamedKey::PageUp),
        "pagedown" | "page_down" => Some(NamedKey::PageDown),
        "home" => Some(NamedKey::Home),
        "end" => Some(NamedKey::End),
        "insert" => Some(NamedKey::Insert),
        "delete" => Some(NamedKey::Delete),
        "left" | "arrowleft" => Some(NamedKey::ArrowLeft),
        "right" | "arrowright" => Some(NamedKey::ArrowRight),
        "up" | "arrowup" => Some(NamedKey::ArrowUp),
        "down" | "arrowdown" => Some(NamedKey::ArrowDown),
        "f1" => Some(NamedKey::F1),
        "f2" => Some(NamedKey::F2),
        "f3" => Some(NamedKey::F3),
        "f4" => Some(NamedKey::F4),
        "f5" => Some(NamedKey::F5),
        "f6" => Some(NamedKey::F6),
        "f7" => Some(NamedKey::F7),
        "f8" => Some(NamedKey::F8),
        "f9" => Some(NamedKey::F9),
        "f10" => Some(NamedKey::F10),
        "f11" => Some(NamedKey::F11),
        "f12" => Some(NamedKey::F12),
        _ => None,
    };
    if let Some(named) = named {
        return Some(BindKey::Named(named));
    }
    let mut chars = part.chars();
    let ch = chars.next()?;
    chars
        .next()
        .is_none()
        .then(|| BindKey::Char(ch.to_ascii_lowercase()))
}

pub fn parse_action(name: &str) -> Result<Option<Action>, String> {
    let action = match name.to_ascii_lowercase().as_str() {
        "none" | "unbind" => return Ok(None),
        "new_tab" => Action::NewTab,
        "close_tab" => Action::CloseTab,
        "next_tab" => Action::NextTab,
        "prev_tab" | "previous_tab" => Action::PreviousTab,
        "copy" => Action::Copy,
        "paste" => Action::Paste,
        "prev_prompt" | "previous_prompt" => Action::PreviousPrompt,
        "next_prompt" => Action::NextPrompt,
        "copy_output" => Action::CopyOutput,
        "toggle_fold" => Action::ToggleFold,
        "search" => Action::Search,
        "page_up" => Action::PageUp,
        "page_down" => Action::PageDown,
        "settings" | "open_settings" => Action::OpenSettings,
        "zoom_in" => Action::ZoomIn,
        "zoom_out" => Action::ZoomOut,
        "zoom_reset" => Action::ZoomReset,
        "split_right" => Action::SplitRight,
        "split_down" => Action::SplitDown,
        "close_pane" => Action::ClosePane,
        "focus_left" => Action::FocusLeft,
        "focus_right" => Action::FocusRight,
        "focus_up" => Action::FocusUp,
        "focus_down" => Action::FocusDown,
        "resize_left" => Action::ResizeLeft,
        "resize_right" => Action::ResizeRight,
        "resize_up" => Action::ResizeUp,
        "resize_down" => Action::ResizeDown,
        other => {
            if let Some(digit) = other
                .strip_prefix("tab_")
                .and_then(|d| d.parse::<usize>().ok())
                && (1..=9).contains(&digit)
            {
                return Ok(Some(Action::SelectTab(digit - 1)));
            }
            return Err(format!("unknown action '{name}'"));
        }
    };
    Ok(Some(action))
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
            Some((at, seen, count)) if seen == cell && now.duration_since(at) < CLICK_WINDOW => {
                count % 3 + 1
            }
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

pub fn resolve_binding(key: &Key, mods: ModifiersState, custom: &[Keybind]) -> Option<Action> {
    let active_mods = mods
        & (ModifiersState::CONTROL
            | ModifiersState::SHIFT
            | ModifiersState::ALT
            | ModifiersState::SUPER);
    for rule in custom.iter().rev() {
        if rule.mods == active_mods && key_matches(&rule.key, key, active_mods.shift_key()) {
            return rule.action;
        }
    }
    binding(key, mods)
}

fn unshifted_symbol(ch: char) -> Option<char> {
    match ch {
        '<' => Some(','),
        '>' => Some('.'),
        '+' => Some('='),
        '_' => Some('-'),
        ')' => Some('0'),
        '!' => Some('1'),
        '@' => Some('2'),
        '#' => Some('3'),
        '$' => Some('4'),
        '%' => Some('5'),
        '^' => Some('6'),
        '&' => Some('7'),
        '*' => Some('8'),
        '(' => Some('9'),
        '?' => Some('/'),
        _ => None,
    }
}

fn key_matches(target: &BindKey, key: &Key, shift: bool) -> bool {
    match (target, key) {
        (BindKey::Named(a), Key::Named(b)) => a == b,
        (BindKey::Char(a), Key::Character(text)) => {
            let mut chars = text.chars();
            let Some(ch) = chars.next() else {
                return false;
            };
            if chars.next().is_some() {
                return false;
            }
            ch.to_ascii_lowercase() == *a || (shift && unshifted_symbol(ch) == Some(*a))
        }
        _ => false,
    }
}

pub fn binding(key: &Key, mods: ModifiersState) -> Option<Action> {
    let shift = mods.shift_key();
    let ctrl_alt_shift = ModifiersState::CONTROL | ModifiersState::ALT | ModifiersState::SHIFT;
    let ctrl_shift = ModifiersState::CONTROL | ModifiersState::SHIFT;
    match key {
        Key::Named(NamedKey::Tab) if mods.control_key() => Some(if shift {
            Action::PreviousTab
        } else {
            Action::NextTab
        }),
        Key::Named(NamedKey::PageUp) if shift => Some(Action::PageUp),
        Key::Named(NamedKey::PageDown) if shift => Some(Action::PageDown),
        Key::Named(NamedKey::ArrowLeft) if mods == ctrl_alt_shift => Some(Action::ResizeLeft),
        Key::Named(NamedKey::ArrowRight) if mods == ctrl_alt_shift => Some(Action::ResizeRight),
        Key::Named(NamedKey::ArrowUp) if mods == ctrl_alt_shift => Some(Action::ResizeUp),
        Key::Named(NamedKey::ArrowDown) if mods == ctrl_alt_shift => Some(Action::ResizeDown),
        Key::Named(NamedKey::ArrowLeft) if mods == ctrl_shift => Some(Action::FocusLeft),
        Key::Named(NamedKey::ArrowRight) if mods == ctrl_shift => Some(Action::FocusRight),
        Key::Named(NamedKey::ArrowUp) if mods == ctrl_shift => Some(Action::FocusUp),
        Key::Named(NamedKey::ArrowDown) if mods == ctrl_shift => Some(Action::FocusDown),
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
                "d" => Some(Action::SplitRight),
                "e" => Some(Action::SplitDown),
                "q" => Some(Action::ClosePane),
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

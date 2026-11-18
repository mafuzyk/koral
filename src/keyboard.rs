use alacritty_terminal::term::TermMode;
use winit::keyboard::{Key, ModifiersState, NamedKey};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Press,
    Repeat,
    Release,
}

pub struct KeyPress<'a> {
    pub key: &'a Key,
    pub unshifted: &'a Key,
    pub text: Option<&'a str>,
    pub mods: ModifiersState,
    pub stage: Stage,
}

/// How a key is written in the protocol: a key code, or the fixed form of a legacy key.
enum Code {
    Unicode(u32),
    Letter(char),
    Tilde(u32),
}

/// The kitty form of a key. None means it is not one: the caller uses `input::encode`.
// shortcut: keypad and media keys use their legacy form, and modifier keys always report
// their left code. Add them if an app needs them.
pub fn encode(press: &KeyPress, mode: TermMode) -> Option<Vec<u8>> {
    if !mode.intersects(TermMode::KITTY_KEYBOARD_PROTOCOL) {
        return None;
    }
    let all_keys = mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC);
    let events = mode.contains(TermMode::REPORT_EVENT_TYPES);
    if press.stage != Stage::Press && !events {
        return None;
    }

    let code = code(press.unshifted, all_keys)?;
    let chorded = press.mods.intersects(ModifiersState::CONTROL | ModifiersState::ALT | ModifiersState::SUPER);
    let quiet = match press.unshifted {
        Key::Named(NamedKey::Escape) => false,
        Key::Named(NamedKey::Enter | NamedKey::Tab | NamedKey::Backspace) => {
            !chorded && !press.mods.shift_key() || press.stage == Stage::Release
        }
        Key::Named(NamedKey::Space) | Key::Character(_) => !chorded,
        _ => press.mods.is_empty() && press.stage == Stage::Press,
    };
    if quiet && !all_keys {
        return None;
    }

    let mods = 1 + u8::from(press.mods.shift_key())
        + 2 * u8::from(press.mods.alt_key())
        + 4 * u8::from(press.mods.control_key())
        + 8 * u8::from(press.mods.super_key());
    let stage = match press.stage {
        Stage::Press => "",
        Stage::Repeat if events => ":2",
        Stage::Release if events => ":3",
        _ => "",
    };
    let state = if mods == 1 && stage.is_empty() { String::new() } else { format!("{mods}{stage}") };

    Some(
        match code {
            Code::Letter(letter) if state.is_empty() => format!("\x1b[{letter}"),
            Code::Letter(letter) => format!("\x1b[1;{state}{letter}"),
            Code::Tilde(number) if state.is_empty() => format!("\x1b[{number}~"),
            Code::Tilde(number) => format!("\x1b[{number};{state}~"),
            Code::Unicode(number) => {
                let key = match alternate(press, mode) {
                    Some(shifted) => format!("{number}:{shifted}"),
                    None => number.to_string(),
                };
                match associated(press, mode) {
                    Some(text) => format!("\x1b[{key};{state};{text}u"),
                    None if state.is_empty() => format!("\x1b[{key}u"),
                    None => format!("\x1b[{key};{state}u"),
                }
            }
        }
        .into_bytes(),
    )
}

fn alternate(press: &KeyPress, mode: TermMode) -> Option<u32> {
    if !mode.contains(TermMode::REPORT_ALTERNATE_KEYS) || !press.mods.shift_key() || press.key == press.unshifted {
        return None;
    }
    let Key::Character(text) = press.key else {
        return None;
    };
    single(text)
}

fn associated(press: &KeyPress, mode: TermMode) -> Option<String> {
    let wanted = TermMode::REPORT_ALL_KEYS_AS_ESC | TermMode::REPORT_ASSOCIATED_TEXT;
    let typed = !press.mods.intersects(ModifiersState::CONTROL | ModifiersState::ALT | ModifiersState::SUPER);
    if !mode.contains(wanted) || press.stage == Stage::Release || !typed {
        return None;
    }
    let text = press.text.filter(|text| !text.is_empty() && !text.chars().any(char::is_control))?;
    Some(text.chars().map(|ch| (ch as u32).to_string()).collect::<Vec<_>>().join(":"))
}

fn single(text: &str) -> Option<u32> {
    let mut chars = text.chars();
    chars.next().filter(|_| chars.next().is_none()).map(u32::from)
}

fn code(key: &Key, all_keys: bool) -> Option<Code> {
    let named = match key {
        Key::Character(text) => return single(&text.to_lowercase()).map(Code::Unicode),
        Key::Named(named) => named,
        _ => return None,
    };
    Some(match named {
        NamedKey::Escape => Code::Unicode(27),
        NamedKey::Enter => Code::Unicode(13),
        NamedKey::Tab => Code::Unicode(9),
        NamedKey::Backspace => Code::Unicode(127),
        NamedKey::Space => Code::Unicode(32),
        NamedKey::ArrowUp => Code::Letter('A'),
        NamedKey::ArrowDown => Code::Letter('B'),
        NamedKey::ArrowRight => Code::Letter('C'),
        NamedKey::ArrowLeft => Code::Letter('D'),
        NamedKey::End => Code::Letter('F'),
        NamedKey::Home => Code::Letter('H'),
        NamedKey::F1 => Code::Letter('P'),
        NamedKey::F2 => Code::Letter('Q'),
        NamedKey::F3 => Code::Letter('R'),
        NamedKey::F4 => Code::Letter('S'),
        NamedKey::Insert => Code::Tilde(2),
        NamedKey::Delete => Code::Tilde(3),
        NamedKey::PageUp => Code::Tilde(5),
        NamedKey::PageDown => Code::Tilde(6),
        NamedKey::F5 => Code::Tilde(15),
        NamedKey::F6 => Code::Tilde(17),
        NamedKey::F7 => Code::Tilde(18),
        NamedKey::F8 => Code::Tilde(19),
        NamedKey::F9 => Code::Tilde(20),
        NamedKey::F10 => Code::Tilde(21),
        NamedKey::F11 => Code::Tilde(23),
        NamedKey::F12 => Code::Tilde(24),
        NamedKey::Shift if all_keys => Code::Unicode(57441),
        NamedKey::Control if all_keys => Code::Unicode(57442),
        NamedKey::Alt if all_keys => Code::Unicode(57443),
        NamedKey::Super if all_keys => Code::Unicode(57444),
        _ => return None,
    })
}

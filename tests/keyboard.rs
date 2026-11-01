use alacritty_terminal::term::TermMode;
use koral::keyboard::{KeyPress, Stage, encode};
use winit::keyboard::{Key, ModifiersState, NamedKey, SmolStr};

const D: TermMode = TermMode::DISAMBIGUATE_ESC_CODES;
const A: TermMode = TermMode::REPORT_ALL_KEYS_AS_ESC;
const E: TermMode = TermMode::REPORT_EVENT_TYPES;
const L: TermMode = TermMode::REPORT_ALTERNATE_KEYS;
const T: TermMode = TermMode::REPORT_ASSOCIATED_TEXT;
const CTRL: ModifiersState = ModifiersState::CONTROL;
const ALT: ModifiersState = ModifiersState::ALT;
const SHIFT: ModifiersState = ModifiersState::SHIFT;
const NONE: ModifiersState = ModifiersState::empty();

fn letter(text: &str) -> Key {
    Key::Character(SmolStr::new(text))
}

fn run(key: Key, unshifted: Key, text: Option<&str>, mods: ModifiersState, stage: Stage, mode: TermMode) -> Option<String> {
    let press = KeyPress { key: &key, unshifted: &unshifted, text, mods, stage };
    encode(&press, mode).map(|bytes| String::from_utf8(bytes).unwrap())
}

fn press(key: Key, text: Option<&str>, mods: ModifiersState, mode: TermMode) -> Option<String> {
    run(key.clone(), key, text, mods, Stage::Press, mode)
}

fn named(key: NamedKey) -> Key {
    Key::Named(key)
}

#[test]
fn escape_is_disambiguated() {
    assert_eq!(press(named(NamedKey::Escape), None, NONE, D).as_deref(), Some("\x1b[27u"));
}

#[test]
fn ctrl_letter_is_disambiguated() {
    assert_eq!(press(letter("a"), None, CTRL, D).as_deref(), Some("\x1b[97;5u"));
}

#[test]
fn alt_letter_is_disambiguated() {
    assert_eq!(press(letter("a"), Some("a"), ALT, D).as_deref(), Some("\x1b[97;3u"));
}

#[test]
fn plain_text_stays_text() {
    assert_eq!(press(letter("a"), Some("a"), NONE, D), None);
}

#[test]
fn shifted_text_stays_text() {
    assert_eq!(run(letter("A"), letter("a"), Some("A"), SHIFT, Stage::Press, D), None);
}

#[test]
fn accents_stay_text() {
    assert_eq!(press(letter("ç"), Some("ç"), NONE, D), None);
}

#[test]
fn plain_enter_stays_legacy() {
    assert_eq!(press(named(NamedKey::Enter), Some("\r"), NONE, D), None);
}

#[test]
fn shift_enter_is_reported() {
    assert_eq!(press(named(NamedKey::Enter), Some("\r"), SHIFT, D).as_deref(), Some("\x1b[13;2u"));
}

#[test]
fn arrows_keep_their_form() {
    assert_eq!(press(named(NamedKey::ArrowUp), None, CTRL, D).as_deref(), Some("\x1b[1;5A"));
}

#[test]
fn a_plain_arrow_stays_legacy() {
    assert_eq!(press(named(NamedKey::ArrowUp), None, NONE, D), None);
}

#[test]
fn page_keys_keep_their_form() {
    assert_eq!(press(named(NamedKey::PageUp), None, CTRL, D).as_deref(), Some("\x1b[5;5~"));
}

#[test]
fn all_keys_report_letters() {
    assert_eq!(press(letter("a"), Some("a"), NONE, A).as_deref(), Some("\x1b[97u"));
}

#[test]
fn all_keys_report_modifier_keys() {
    assert_eq!(press(named(NamedKey::Shift), None, SHIFT, A).as_deref(), Some("\x1b[57441;2u"));
}

#[test]
fn modifier_keys_stay_quiet_without_all_keys() {
    assert_eq!(press(named(NamedKey::Shift), None, SHIFT, D), None);
}

#[test]
fn associated_text_is_added() {
    assert_eq!(press(letter("a"), Some("a"), NONE, A | T).as_deref(), Some("\x1b[97;;97u"));
}

#[test]
fn alternate_key_is_added() {
    assert_eq!(run(letter("A"), letter("a"), Some("A"), SHIFT, Stage::Press, A | L).as_deref(), Some("\x1b[97:65;2u"));
}

#[test]
fn release_is_reported() {
    assert_eq!(run(letter("a"), letter("a"), None, NONE, Stage::Release, A | E).as_deref(), Some("\x1b[97;1:3u"));
}

#[test]
fn repeat_is_reported() {
    assert_eq!(run(letter("a"), letter("a"), Some("a"), NONE, Stage::Repeat, A | E).as_deref(), Some("\x1b[97;1:2u"));
}

#[test]
fn an_arrow_reports_its_release() {
    let up = named(NamedKey::ArrowUp);
    assert_eq!(run(up.clone(), up, None, NONE, Stage::Release, D | E).as_deref(), Some("\x1b[1;1:3A"));
}

#[test]
fn enter_release_needs_all_keys() {
    let enter = named(NamedKey::Enter);
    assert_eq!(run(enter.clone(), enter, None, NONE, Stage::Release, D | E), None);
}

#[test]
fn a_release_needs_event_types() {
    assert_eq!(run(letter("a"), letter("a"), None, CTRL, Stage::Release, D), None);
}

#[test]
fn legacy_mode_sends_nothing_new() {
    assert_eq!(press(letter("a"), None, CTRL, TermMode::empty()), None);
}

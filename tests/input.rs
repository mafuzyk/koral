use alacritty_terminal::term::TermMode;
use std::time::{Duration, Instant};

use alacritty_terminal::selection::SelectionType;
use koral::input::{Clicks, Mouse, encode, mouse_report, paste, selection_kind, zoomed};
use winit::keyboard::{Key, ModifiersState, NamedKey};

const NONE: ModifiersState = ModifiersState::empty();
const CTRL: ModifiersState = ModifiersState::CONTROL;
const ALT: ModifiersState = ModifiersState::ALT;
const SHIFT: ModifiersState = ModifiersState::SHIFT;

fn named(key: NamedKey, mods: ModifiersState, mode: TermMode) -> Option<Vec<u8>> {
    encode(&Key::Named(key), None, mods, mode)
}

fn character(text: &str, mods: ModifiersState) -> Option<Vec<u8>> {
    encode(&Key::Character(text.into()), Some(text), mods, TermMode::empty())
}

fn plain(key: NamedKey) -> Option<Vec<u8>> {
    named(key, NONE, TermMode::empty())
}

#[test]
fn text_is_sent_as_utf8() {
    assert_eq!(character("é", NONE).unwrap(), "é".as_bytes());
}

#[test]
fn control_letters_become_c0_codes() {
    assert_eq!(character("c", CTRL).unwrap(), vec![3]);
    assert_eq!(character("[", CTRL).unwrap(), vec![0x1b]);
}

#[test]
fn control_space_is_nul() {
    assert_eq!(named(NamedKey::Space, CTRL, TermMode::empty()).unwrap(), vec![0]);
}

#[test]
fn alt_prefixes_escape() {
    assert_eq!(character("x", ALT).unwrap(), b"\x1bx");
}

#[test]
fn editing_keys_have_their_codes() {
    assert_eq!(plain(NamedKey::Enter).unwrap(), b"\r");
    assert_eq!(plain(NamedKey::Tab).unwrap(), b"\t");
    assert_eq!(plain(NamedKey::Backspace).unwrap(), b"\x7f");
    assert_eq!(plain(NamedKey::Escape).unwrap(), b"\x1b");
    assert_eq!(plain(NamedKey::Delete).unwrap(), b"\x1b[3~");
    assert_eq!(plain(NamedKey::PageDown).unwrap(), b"\x1b[6~");
}

#[test]
fn shift_tab_is_backtab() {
    assert_eq!(named(NamedKey::Tab, SHIFT, TermMode::empty()).unwrap(), b"\x1b[Z");
}

#[test]
fn arrows_follow_application_cursor_mode() {
    assert_eq!(plain(NamedKey::ArrowUp).unwrap(), b"\x1b[A");
    assert_eq!(named(NamedKey::ArrowUp, NONE, TermMode::APP_CURSOR).unwrap(), b"\x1bOA");
    assert_eq!(named(NamedKey::Home, NONE, TermMode::APP_CURSOR).unwrap(), b"\x1bOH");
}

#[test]
fn modified_arrows_carry_the_modifier_code() {
    assert_eq!(named(NamedKey::ArrowLeft, CTRL, TermMode::empty()).unwrap(), b"\x1b[1;5D");
    assert_eq!(named(NamedKey::ArrowRight, SHIFT | ALT, TermMode::empty()).unwrap(), b"\x1b[1;4C");
}

#[test]
fn function_keys_have_their_codes() {
    assert_eq!(plain(NamedKey::F1).unwrap(), b"\x1bOP");
    assert_eq!(plain(NamedKey::F5).unwrap(), b"\x1b[15~");
    assert_eq!(plain(NamedKey::F12).unwrap(), b"\x1b[24~");
    assert_eq!(named(NamedKey::F5, CTRL, TermMode::empty()).unwrap(), b"\x1b[15;5~");
}

#[test]
fn bare_modifiers_send_nothing() {
    assert_eq!(plain(NamedKey::Shift), None);
}

#[test]
fn paste_turns_newlines_into_returns_without_bracketed_mode() {
    assert_eq!(paste("a\r\nb\nc", TermMode::empty()), b"a\rb\rc");
}

#[test]
fn bracketed_paste_wraps_the_text() {
    assert_eq!(paste("a\nb", TermMode::BRACKETED_PASTE), b"\x1b[200~a\nb\x1b[201~");
}

#[test]
fn bracketed_paste_cannot_be_closed_by_the_pasted_text() {
    let out = paste("x\x1b[201~rm -rf /\n", TermMode::BRACKETED_PASTE);
    assert_eq!(out, b"\x1b[200~x[201~rm -rf /\n\x1b[201~");
}

#[test]
fn bracketed_paste_drops_the_c1_control_introducer() {
    let out = paste("x\u{9b}201~y", TermMode::BRACKETED_PASTE);
    assert_eq!(out, b"\x1b[200~x201~y\x1b[201~");
}

fn press(button: u8, column: usize, line: usize) -> Mouse {
    Mouse { button, column, line, pressed: true, mods: NONE }
}

#[test]
fn mouse_reports_nothing_unless_the_app_asked() {
    assert_eq!(mouse_report(press(0, 1, 1), TermMode::empty()), None);
}

#[test]
fn sgr_mouse_is_one_based_and_marks_release() {
    let mode = TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE;
    assert_eq!(mouse_report(press(0, 4, 2), mode).unwrap(), b"\x1b[<0;5;3M");
    let release = Mouse { pressed: false, ..press(0, 4, 2) };
    assert_eq!(mouse_report(release, mode).unwrap(), b"\x1b[<0;5;3m");
}

#[test]
fn sgr_mouse_adds_modifier_bits() {
    let mode = TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE;
    let mouse = Mouse { mods: CTRL | ALT, ..press(64, 0, 0) };
    assert_eq!(mouse_report(mouse, mode).unwrap(), b"\x1b[<88;1;1M");
}

#[test]
fn legacy_mouse_offsets_values_by_32() {
    let mode = TermMode::MOUSE_REPORT_CLICK;
    assert_eq!(mouse_report(press(0, 1, 2), mode).unwrap(), vec![0x1b, b'[', b'M', 32, 34, 35]);
    let release = Mouse { pressed: false, ..press(2, 0, 0) };
    assert_eq!(mouse_report(release, mode).unwrap(), vec![0x1b, b'[', b'M', 35, 33, 33]);
}

#[test]
fn legacy_mouse_cannot_report_past_its_range() {
    assert_eq!(mouse_report(press(0, 300, 0), TermMode::MOUSE_REPORT_CLICK), None);
}

#[test]
fn ctrl_cedilla_stays_silent_in_legacy_mode() {
    let cedilla = Key::Character("ç".into());
    assert_eq!(encode(&cedilla, None, ModifiersState::CONTROL, TermMode::empty()), None);
}

#[test]
fn clicks_on_one_cell_count_up() {
    let (mut clicks, start) = (Clicks::default(), Instant::now());
    let at = |ms| start + Duration::from_millis(ms);
    let counts: Vec<u8> = [0, 100, 200, 300].map(|ms| clicks.press((1, 2), at(ms))).to_vec();
    assert_eq!(counts, [1, 2, 3, 1]);
}

#[test]
fn a_slow_click_starts_again() {
    let (mut clicks, start) = (Clicks::default(), Instant::now());
    clicks.press((1, 2), start);
    assert_eq!(clicks.press((1, 2), start + Duration::from_millis(500)), 1);
}

#[test]
fn a_click_elsewhere_starts_again() {
    let (mut clicks, start) = (Clicks::default(), Instant::now());
    clicks.press((1, 2), start);
    assert_eq!(clicks.press((1, 3), start + Duration::from_millis(100)), 1);
}

#[test]
fn click_counts_choose_the_selection() {
    assert_eq!(selection_kind(1, NONE), SelectionType::Simple);
    assert_eq!(selection_kind(2, NONE), SelectionType::Semantic);
    assert_eq!(selection_kind(3, NONE), SelectionType::Lines);
}

#[test]
fn ctrl_alt_selects_a_block() {
    assert_eq!(selection_kind(2, CTRL | ALT), SelectionType::Block);
}

#[test]
fn zoom_stays_in_range() {
    assert_eq!(zoomed(71.5, 1.0), 72.0);
    assert_eq!(zoomed(6.5, -1.0), 6.0);
    assert_eq!(zoomed(13.5, 1.0), 14.5);
}

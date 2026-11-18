use koral::input::{Action, binding};
use winit::keyboard::{Key, ModifiersState, NamedKey};

const CTRL_SHIFT: ModifiersState = ModifiersState::CONTROL.union(ModifiersState::SHIFT);

fn letter(text: &str, mods: ModifiersState) -> Option<Action> {
    binding(&Key::Character(text.into()), mods)
}

#[test]
fn tab_shortcuts_match_with_shift_held() {
    assert_eq!(letter("T", CTRL_SHIFT), Some(Action::NewTab));
    assert_eq!(letter("W", CTRL_SHIFT), Some(Action::CloseTab));
}

#[test]
fn control_tab_cycles() {
    let tab = Key::Named(NamedKey::Tab);
    assert_eq!(binding(&tab, ModifiersState::CONTROL), Some(Action::NextTab));
    assert_eq!(binding(&tab, CTRL_SHIFT), Some(Action::PreviousTab));
}

#[test]
fn alt_digits_pick_a_tab_from_one() {
    assert_eq!(letter("1", ModifiersState::ALT), Some(Action::SelectTab(0)));
    assert_eq!(letter("9", ModifiersState::ALT), Some(Action::SelectTab(8)));
    assert_eq!(letter("0", ModifiersState::ALT), None);
}

#[test]
fn clipboard_uses_control_shift() {
    assert_eq!(letter("C", CTRL_SHIFT), Some(Action::Copy));
    assert_eq!(letter("V", CTRL_SHIFT), Some(Action::Paste));
}

#[test]
fn block_actions_have_keys() {
    assert_eq!(letter("Z", CTRL_SHIFT), Some(Action::PreviousPrompt));
    assert_eq!(letter("X", CTRL_SHIFT), Some(Action::NextPrompt));
    assert_eq!(letter("G", CTRL_SHIFT), Some(Action::CopyOutput));
    assert_eq!(letter("F", CTRL_SHIFT), Some(Action::ToggleFold));
}

#[test]
fn shift_page_keys_scroll() {
    let up = Key::Named(NamedKey::PageUp);
    assert_eq!(binding(&up, ModifiersState::SHIFT), Some(Action::PageUp));
}

#[test]
fn keys_meant_for_the_app_pass_through() {
    assert_eq!(letter("c", ModifiersState::CONTROL), None);
    assert_eq!(letter("t", ModifiersState::empty()), None);
    assert_eq!(binding(&Key::Named(NamedKey::PageUp), ModifiersState::empty()), None);
}

#[test]
fn zoom_keys_work_with_or_without_the_shifted_character() {
    assert_eq!(letter("=", ModifiersState::CONTROL), Some(Action::ZoomIn));
    assert_eq!(letter("+", ModifiersState::CONTROL), Some(Action::ZoomIn));
    assert_eq!(letter("-", ModifiersState::CONTROL), Some(Action::ZoomOut));
    assert_eq!(letter("=", CTRL_SHIFT), Some(Action::ZoomIn));
    assert_eq!(letter("+", CTRL_SHIFT), Some(Action::ZoomIn));
    assert_eq!(letter("-", CTRL_SHIFT), Some(Action::ZoomOut));
    assert_eq!(letter("_", CTRL_SHIFT), Some(Action::ZoomOut));
    assert_eq!(letter("0", CTRL_SHIFT), Some(Action::ZoomReset));
    assert_eq!(letter(")", CTRL_SHIFT), Some(Action::ZoomReset));
}

#[test]
fn control_shift_s_searches() {
    assert_eq!(letter("S", CTRL_SHIFT), Some(Action::Search));
}

use koral::input::{
    Action, BindKey, Keybind, binding, parse_action, parse_shortcut, resolve_binding,
};
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
    assert_eq!(
        binding(&tab, ModifiersState::CONTROL),
        Some(Action::NextTab)
    );
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
    assert_eq!(letter("Y", CTRL_SHIFT), Some(Action::DiffBlocks));
    assert_eq!(letter("B", CTRL_SHIFT), Some(Action::SearchBlock));
    assert_eq!(letter("L", CTRL_SHIFT), Some(Action::JumpLastError));
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
    assert_eq!(
        binding(&Key::Named(NamedKey::PageUp), ModifiersState::empty()),
        None
    );
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

#[test]
fn split_focus_and_resize_defaults_match() {
    let ctrl_alt_shift = CTRL_SHIFT | ModifiersState::ALT;
    assert_eq!(letter("D", CTRL_SHIFT), Some(Action::SplitRight));
    assert_eq!(letter("E", CTRL_SHIFT), Some(Action::SplitDown));
    assert_eq!(letter("Q", CTRL_SHIFT), Some(Action::ClosePane));
    assert_eq!(
        binding(&Key::Named(NamedKey::ArrowLeft), CTRL_SHIFT),
        Some(Action::FocusLeft)
    );
    assert_eq!(
        binding(&Key::Named(NamedKey::ArrowRight), ctrl_alt_shift),
        Some(Action::ResizeRight)
    );
}

#[test]
fn custom_bindings_override_or_unbind_defaults() {
    let (m_plus, k_plus) = parse_shortcut("ctrl++").unwrap();
    assert_eq!(m_plus, ModifiersState::CONTROL);
    assert_eq!(k_plus, BindKey::Char('+'));

    let custom = vec![
        Keybind {
            mods: CTRL_SHIFT,
            key: BindKey::Char('n'),
            action: parse_action("new_tab").unwrap(),
        },
        Keybind {
            mods: CTRL_SHIFT,
            key: BindKey::Char('t'),
            action: parse_action("unbind").unwrap(),
        },
    ];
    assert_eq!(
        resolve_binding(&Key::Character("N".into()), CTRL_SHIFT, &custom),
        Some(Action::NewTab)
    );
    assert_eq!(
        resolve_binding(&Key::Character("T".into()), CTRL_SHIFT, &custom),
        None
    );
    assert_eq!(
        resolve_binding(&Key::Character("W".into()), CTRL_SHIFT, &custom),
        Some(Action::CloseTab)
    );
}

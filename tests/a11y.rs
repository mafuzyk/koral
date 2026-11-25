use accesskit::Role;
use koral::a11y::{MAX_VISIBLE_LINES, ROOT_ID, TERMINAL_ID, build_tree};

#[test]
fn build_tree_exposes_window_terminal_and_visible_lines() {
    let lines = vec!["$ echo hi".to_string(), "hi".to_string()];
    let update = build_tree("koral", "bash", &lines, true);
    assert_eq!(update.focus, TERMINAL_ID);
    assert_eq!(update.tree.as_ref().unwrap().root, ROOT_ID);
    assert_eq!(
        update.tree.as_ref().unwrap().toolkit_name.as_deref(),
        Some("koral")
    );
    assert_eq!(update.nodes.len(), 4);
    assert_eq!(update.nodes[0].1.role(), Role::Window);
    assert_eq!(update.nodes[0].1.label(), Some("koral"));
    assert_eq!(update.nodes[1].1.role(), Role::Terminal);
    assert_eq!(update.nodes[1].1.label(), Some("bash"));
    assert_eq!(update.nodes[2].1.role(), Role::TextRun);
    assert_eq!(update.nodes[2].1.value(), Some("$ echo hi"));
    assert_eq!(update.nodes[3].1.value(), Some("hi"));
}

#[test]
fn build_tree_caps_lines_and_handles_unfocused_root() {
    let many: Vec<String> = (0..400).map(|i| format!("line {i}")).collect();
    let update = build_tree("koral", "tab", &many, false);
    assert_eq!(update.focus, ROOT_ID);
    assert_eq!(update.nodes.len(), MAX_VISIBLE_LINES + 2);
}

use accesskit::{Node, NodeId, Role, TreeId, TreeInfo, TreeUpdate};
use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::Column;
use alacritty_terminal::term::Term;

use crate::frame;
use crate::view::{Row, without_anchors};

pub const ROOT_ID: NodeId = NodeId(0);
pub const TERMINAL_ID: NodeId = NodeId(1);
pub const MAX_VISIBLE_LINES: usize = 256;

/// Builds a bounded AccessKit snapshot of the active window and terminal pane.
pub fn build_tree(
    window_title: &str,
    active_tab_title: &str,
    visible_lines: &[String],
    focused: bool,
) -> TreeUpdate {
    let mut line_ids = Vec::with_capacity(visible_lines.len().min(MAX_VISIBLE_LINES));
    let mut line_nodes = Vec::with_capacity(visible_lines.len().min(MAX_VISIBLE_LINES));

    for (idx, text) in visible_lines.iter().take(MAX_VISIBLE_LINES).enumerate() {
        let id = NodeId((idx + 2) as u64);
        let mut line = Node::new(Role::TextRun);
        line.set_value(text.as_str());
        line_nodes.push((id, line));
        line_ids.push(id);
    }

    let mut terminal = Node::new(Role::Terminal);
    terminal.set_label(active_tab_title);
    terminal.set_children(line_ids);

    let mut root = Node::new(Role::Window);
    root.set_label(window_title);
    root.set_children(vec![TERMINAL_ID]);

    let mut nodes = Vec::with_capacity(line_nodes.len() + 2);
    nodes.push((ROOT_ID, root));
    nodes.push((TERMINAL_ID, terminal));
    nodes.extend(line_nodes);

    let mut info = TreeInfo::new(ROOT_ID);
    info.toolkit_name = Some("koral".to_string());
    info.toolkit_version = Some(env!("CARGO_PKG_VERSION").to_string());

    TreeUpdate {
        nodes,
        tree: Some(info),
        tree_id: TreeId::ROOT,
        focus: if focused { TERMINAL_ID } else { ROOT_ID },
    }
}

/// Extracts plain visible lines for the active viewport without private-use anchor codepoints.
pub fn visible_lines<L: EventListener>(
    term: &Term<L>,
    rows: &[Row],
    scroll: usize,
    lines: usize,
) -> Vec<String> {
    let top = frame::visible_top(rows, lines, scroll);
    let end = rows.len().min(top + lines);
    let cols = term.columns();
    let grid = term.grid();
    rows[top..end]
        .iter()
        .map(|row| {
            let mut text = String::with_capacity(cols);
            for col in 0..cols {
                text.push(grid[row.line][Column(col)].c);
            }
            let cleaned = without_anchors(&text);
            cleaned.trim_end().to_string()
        })
        .collect()
}

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::{Dimensions, Grid};
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::Term;
use alacritty_terminal::term::cell::{Cell, Flags};

use crate::blocks::{Activity, BlockStore};

const ANCHOR_BASE: u32 = 0x10_0000;

/// A visible row. `hidden` counts the folded lines that belong under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub line: Line,
    pub block: Option<u32>,
    pub hidden: usize,
}

pub fn anchor_char(id: u32) -> char {
    char::from_u32(ANCHOR_BASE + id).expect("block ids stay inside plane 16")
}

/// Grid text for the user: selections and copies must never carry anchors.
pub fn without_anchors(text: &str) -> String {
    text.chars().filter(|ch| (*ch as u32) < ANCHOR_BASE).collect()
}

// shortcut: walks every line on each call, cache per frame if profiling asks.
pub fn rows<L: EventListener>(term: &Term<L>, blocks: &BlockStore) -> Vec<Row> {
    let grid = term.grid();
    let mut rows = Vec::new();
    let mut fold_into = None;
    let mut in_command = false;

    for number in grid.topmost_line().0..=grid.bottommost_line().0 {
        let line = Line(number);
        if let Some(id) = block_at(grid, line, blocks) {
            rows.push(Row { line, block: Some(id), hidden: 0 });
            let folded = blocks.get(id).is_some_and(|block| block.folded);
            fold_into = folded.then_some(rows.len() - 1);
            in_command = wraps(grid, line);
        } else if in_command {
            rows.push(Row { line, block: None, hidden: 0 });
            in_command = wraps(grid, line);
        } else if let Some(header) = fold_into {
            let row: &mut Row = &mut rows[header];
            row.hidden += 1;
        } else {
            rows.push(Row { line, block: None, hidden: 0 });
        }
    }
    rows
}

/// The running command to show above the viewport once its header scrolled off.
pub fn pinned(rows: &[Row], top: usize, blocks: &BlockStore) -> Option<u32> {
    let header = rows.get(..=top)?.iter().rposition(|row| row.block.is_some())?;
    let id = rows[header].block?;
    let running = blocks.activity() == Activity::Running && blocks.latest()?.id == id;
    (running && header != top).then_some(id)
}

pub fn output_text<L: EventListener>(term: &Term<L>, blocks: &BlockStore, id: u32) -> Option<String> {
    let grid = term.grid();
    let headers: Vec<(Line, u32)> = (grid.topmost_line().0..=grid.bottommost_line().0)
        .filter_map(|number| Some((Line(number), block_at(grid, Line(number), blocks)?)))
        .collect();
    let index = headers.iter().position(|(_, header)| *header == id)?;

    let mut start = headers[index].0;
    while wraps(grid, start) {
        start = Line(start.0 + 1);
    }
    start = Line(start.0 + 1);
    let end = headers.get(index + 1).map_or(grid.cursor.point.line, |(line, _)| Line(line.0 - 1));
    if start > end {
        return Some(String::new());
    }

    let last = Column(grid.columns() - 1);
    let text = term.bounds_to_string(Point::new(start, Column(0)), Point::new(end, last));
    Some(without_anchors(&text).trim_end_matches('\n').to_string())
}

fn block_at(grid: &Grid<Cell>, line: Line, blocks: &BlockStore) -> Option<u32> {
    grid[line][Column(0)]
        .zerowidth()?
        .iter()
        .filter_map(|ch| (*ch as u32).checked_sub(ANCHOR_BASE))
        .find(|id| blocks.get(*id).is_some())
}

fn wraps(grid: &Grid<Cell>, line: Line) -> bool {
    grid[line][Column(grid.columns() - 1)].flags.contains(Flags::WRAPLINE)
}

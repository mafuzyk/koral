use std::collections::HashSet;
use std::sync::Arc;

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::{Dimensions, Grid};
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::Term;
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::vte::ansi::{Color, Rgb};

use crate::blocks::{Activity, BlockStore};
use crate::graphics::{Graphics, Placement};

const ANCHOR_BASE: u32 = 0x10_0000;
// Plane 16 stays free of real text: Nerd Font icons live in plane 15.
const PICTURE_BASE: u32 = ANCHOR_BASE + 0x8000;
// A cell the grid must count as used, so a clear takes an image's whole height.
const PICTURE_FOOT: u32 = 0x7FFF;
pub const PLACEHOLDER_CHAR: char = '\u{10EEEE}';

/// A visible row. `hidden` counts the folded lines that belong under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub line: Line,
    pub block: Option<u32>,
    pub hidden: usize,
}

/// An image on screen. `row` counts from the top of the view and can be
/// negative for one that started above it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placed {
    pub image: u32,
    pub row: isize,
    pub column: usize,
    pub columns: u32,
    pub rows: u32,
    pub crop: [u32; 4],
    /// The placement id, which breaks ties between pictures at the same z-index.
    pub number: u32,
    /// Below zero draws under the text.
    pub z: i32,
}

pub fn picture_char(anchor: u32) -> char {
    char::from_u32(PICTURE_BASE + anchor).expect("picture anchors stay inside plane 16")
}

pub fn picture_anchor(ch: char) -> Option<u32> {
    (ch as u32)
        .checked_sub(PICTURE_BASE)
        .filter(|anchor| *anchor != PICTURE_FOOT)
}

pub fn picture_foot() -> char {
    picture_char(PICTURE_FOOT)
}

fn placeholder_id(color: Color) -> u32 {
    match color {
        Color::Spec(Rgb { r, g, b }) => (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b),
        Color::Indexed(index) => u32::from(index),
        Color::Named(named) => named as u32,
    }
}

pub fn placed<L: EventListener>(
    term: &Term<L>,
    graphics: &Graphics,
    rows: &[Row],
    top: usize,
    lines: usize,
) -> Vec<Placed> {
    let tallest = graphics.tallest();
    if tallest == 0 {
        return Vec::new();
    }
    let grid = term.grid();
    let mut found = Vec::new();
    for (index, row) in rows
        .iter()
        .enumerate()
        .take(top + lines)
        .skip(top.saturating_sub(tallest))
    {
        let row_offset = index as isize - top as isize;
        for column in 0..grid.columns() {
            let cell = &grid[row.line][Column(column)];
            let anchors = cell.zerowidth().unwrap_or(&[]);
            for placement in anchors
                .iter()
                .filter_map(|ch| graphics.placement(picture_anchor(*ch)?))
            {
                if row_offset + placement.rows as isize > 0 {
                    let Placement {
                        image,
                        number,
                        columns,
                        rows,
                        crop,
                        z,
                    } = *placement;
                    found.push(Placed {
                        image,
                        number,
                        row: row_offset,
                        column,
                        columns,
                        rows,
                        crop,
                        z,
                    });
                }
            }
            if cell.c == PLACEHOLDER_CHAR {
                let same = |other: &Cell| {
                    other.c == PLACEHOLDER_CHAR
                        && other.fg == cell.fg
                        && other.underline_color() == cell.underline_color()
                };
                let left_same = column > 0 && same(&grid[row.line][Column(column - 1)]);
                let above_same = index > 0 && same(&grid[rows[index - 1].line][Column(column)]);
                if !left_same && !above_same {
                    let image_id = placeholder_id(cell.fg);
                    let number = cell.underline_color().map_or(0, placeholder_id);
                    if let Some(placement) = graphics.virtual_placement(image_id, number)
                        && row_offset + placement.rows as isize > 0
                    {
                        let Placement {
                            image,
                            number,
                            columns,
                            rows,
                            crop,
                            z,
                        } = *placement;
                        found.push(Placed {
                            image,
                            number,
                            row: row_offset,
                            column,
                            columns,
                            rows,
                            crop,
                            z,
                        });
                    }
                }
            }
        }
    }
    found
}

pub fn anchor_char(id: u32) -> char {
    char::from_u32(ANCHOR_BASE + id).expect("block ids stay inside plane 16")
}

/// Grid text for the user: selections and copies must never carry anchors.
pub fn without_anchors(text: &str) -> String {
    text.chars()
        .filter(|ch| (*ch as u32) < ANCHOR_BASE)
        .collect()
}

pub fn rows<L: EventListener>(term: &Term<L>, blocks: &BlockStore) -> Vec<Row> {
    let grid = term.grid();
    let (top, bottom) = (grid.topmost_line().0, grid.bottommost_line().0);
    let capacity = (bottom - top + 1).max(0) as usize;
    if blocks.is_empty() {
        let mut rows = Vec::with_capacity(capacity);
        for number in top..=bottom {
            rows.push(Row {
                line: Line(number),
                block: None,
                hidden: 0,
            });
        }
        return rows;
    }
    if !blocks.any_folded() {
        let mut rows = Vec::with_capacity(capacity);
        for number in top..=bottom {
            let line = Line(number);
            rows.push(Row {
                line,
                block: block_at(grid, line, blocks),
                hidden: 0,
            });
        }
        return rows;
    }

    let mut rows = Vec::with_capacity(capacity);
    let mut fold_into = None;
    let mut in_command = false;
    let prompts: HashSet<i32> = headers(term, blocks)
        .iter()
        .flat_map(|header| prompt_top(*header, blocks).0..header.0.0)
        .collect();

    for number in top..=bottom {
        let line = Line(number);
        if let Some(id) = block_at(grid, line, blocks) {
            rows.push(Row {
                line,
                block: Some(id),
                hidden: 0,
            });
            let folded = blocks.get(id).is_some_and(|block| block.folded);
            fold_into = folded.then_some(rows.len() - 1);
            in_command = wraps(grid, line);
        } else if in_command {
            rows.push(Row {
                line,
                block: None,
                hidden: 0,
            });
            in_command = wraps(grid, line);
        } else if prompts.contains(&number) {
            rows.push(Row {
                line,
                block: None,
                hidden: 0,
            });
        } else if let Some(header) = fold_into {
            let row: &mut Row = &mut rows[header];
            row.hidden += 1;
        } else {
            rows.push(Row {
                line,
                block: None,
                hidden: 0,
            });
        }
    }
    rows
}

pub fn row_of(rows: &[Row], line: Line) -> Option<usize> {
    rows.binary_search_by_key(&line, |row| row.line).ok()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProjectionKey {
    grid_len: usize,
    blocks_revision: u64,
    grid_revision: u64,
}

/// Caches the projected rows across redraws. A refresh rebuilds only when the grid shape or
/// either revision changes; a viewport request slices the cache without rescanning scrollback.
pub struct RowProjection {
    key: Option<ProjectionKey>,
    cache: Arc<Vec<Row>>,
    hits: u64,
}

impl Default for RowProjection {
    fn default() -> Self {
        Self {
            key: None,
            cache: Arc::new(Vec::new()),
            hits: 0,
        }
    }
}

impl RowProjection {
    /// Returns true when the cached rows were reused.
    pub fn refresh<L: EventListener>(
        &mut self,
        term: &Term<L>,
        blocks: &BlockStore,
        grid_revision: u64,
    ) -> bool {
        let grid = term.grid();
        let key = ProjectionKey {
            grid_len: grid.total_lines(),
            blocks_revision: blocks.revision(),
            grid_revision,
        };
        if self.key == Some(key) {
            self.hits += 1;
            return true;
        }
        let fresh = Arc::new(rows(term, blocks));
        self.cache = fresh;
        self.key = Some(key);
        false
    }

    pub fn rows(&self) -> &[Row] {
        &self.cache
    }

    /// Shared handle to the projected rows, stable across later refreshes.
    pub fn shared(&self) -> Arc<Vec<Row>> {
        Arc::clone(&self.cache)
    }

    /// Number of refreshes that reused the cache instead of projecting again.
    pub fn hits(&self) -> u64 {
        self.hits
    }

    /// The rows visible in a pane at a given scroll offset.
    pub fn viewport(&self, lines: usize, scroll: usize) -> &[Row] {
        let top = crate::frame::visible_top(&self.cache, lines, scroll);
        &self.cache[top..self.cache.len().min(top + lines)]
    }
}

/// The folded block that hides a line.
pub fn block_of(rows: &[Row], line: Line) -> Option<u32> {
    if row_of(rows, line).is_some() {
        return None;
    }
    let header = rows
        .iter()
        .rev()
        .filter(|row| row.line <= line)
        .find(|row| row.block.is_some())?;
    header.block.filter(|_| header.hidden > 0)
}

/// The running command to show above the viewport once its header scrolled off.
pub fn pinned(rows: &[Row], top: usize, blocks: &BlockStore) -> Option<u32> {
    let header = rows
        .get(..=top)?
        .iter()
        .rposition(|row| row.block.is_some())?;
    let id = rows[header].block?;
    let running = blocks.activity() == Activity::Running && blocks.latest()?.id == id;
    (running && header != top).then_some(id)
}

pub fn output_text<L: EventListener>(
    term: &Term<L>,
    blocks: &BlockStore,
    id: u32,
) -> Option<String> {
    let grid = term.grid();
    let headers = headers(term, blocks);
    let index = headers.iter().position(|(_, header)| *header == id)?;

    let mut start = headers[index].0;
    while wraps(grid, start) {
        start = Line(start.0 + 1);
    }
    start = Line(start.0 + 1);
    let end = headers
        .get(index + 1)
        .map_or(grid.cursor.point.line, |next| prompt_top(*next, blocks) - 1);
    if start > end {
        return Some(String::new());
    }

    let last = Column(grid.columns() - 1);
    let text = term.bounds_to_string(Point::new(start, Column(0)), Point::new(end, last));
    Some(without_anchors(&text).trim_end_matches('\n').to_string())
}

fn headers<L: EventListener>(term: &Term<L>, blocks: &BlockStore) -> Vec<(Line, u32)> {
    let grid = term.grid();
    (grid.topmost_line().0..=grid.bottommost_line().0)
        .filter_map(|number| Some((Line(number), block_at(grid, Line(number), blocks)?)))
        .collect()
}

/// Where a block's prompt starts: output above it belongs to the block before.
fn prompt_top((line, id): (Line, u32), blocks: &BlockStore) -> Line {
    line - blocks.get(id).map_or(0, |block| block.prompt_lines)
}

fn block_at(grid: &Grid<Cell>, line: Line, blocks: &BlockStore) -> Option<u32> {
    grid[line][Column(0)]
        .zerowidth()?
        .iter()
        .filter_map(|ch| (*ch as u32).checked_sub(ANCHOR_BASE))
        .find(|id| blocks.get(*id).is_some())
}

fn wraps(grid: &Grid<Cell>, line: Line) -> bool {
    grid[line][Column(grid.columns() - 1)]
        .flags
        .contains(Flags::WRAPLINE)
}

pub fn previous_prompt(rows: &[Row], top: usize) -> Option<usize> {
    rows.get(..top)?.iter().rposition(|row| row.block.is_some())
}

pub fn next_prompt(rows: &[Row], top: usize) -> Option<usize> {
    let offset = rows
        .get(top + 1..)?
        .iter()
        .position(|row| row.block.is_some())?;
    Some(top + 1 + offset)
}

/// The block a fold key acts on: the last finished one above `end`.
pub fn fold_target(rows: &[Row], end: usize, blocks: &BlockStore) -> Option<u32> {
    rows[..end.min(rows.len())]
        .iter()
        .rev()
        .filter_map(|row| row.block)
        .find(|id| blocks.get(*id).is_some_and(|block| block.status.is_some()))
}

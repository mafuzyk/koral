use std::time::Instant;

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::Processor;

use crate::blocks::BlockStore;
use crate::marks::{Piece, Scanner};
use crate::view::anchor_char;

/// One terminal: the grid plus the commands it has shown.
pub struct Session<L> {
    pub term: Term<L>,
    pub blocks: BlockStore,
    parser: Processor,
    scanner: Scanner,
}

impl<L: EventListener> Session<L> {
    pub fn new(term: Term<L>) -> Self {
        Self { term, blocks: BlockStore::default(), parser: Processor::new(), scanner: Scanner::default() }
    }

    pub fn feed(&mut self, bytes: &[u8], now: Instant) {
        let Self { term, blocks, parser, scanner } = self;
        scanner.feed(bytes, &mut |piece| match piece {
            Piece::Bytes(bytes) => {
                parser.advance(term, bytes);
                blocks.observe(bytes);
            }
            Piece::Mark(mark) => {
                if term.mode().contains(TermMode::ALT_SCREEN) {
                    return;
                }
                if let Some(id) = blocks.apply(mark, now) {
                    anchor(term, id);
                }
            }
        });
    }
}

/// Lines have no stable identity: a character on the first cell travels with
/// it through scrollback and reflow. Writing to the cell drops the anchor, so it
/// goes in after the prompt text, and sharky's prompt redraw places it again.
fn anchor<L>(term: &mut Term<L>, id: u32) {
    let grid = term.grid_mut();
    let last = Column(grid.columns() - 1);
    let top = grid.topmost_line();
    let mut line = grid.cursor.point.line;
    while line > top && grid[Line(line.0 - 1)][last].flags.contains(Flags::WRAPLINE) {
        line = Line(line.0 - 1);
    }

    let ch = anchor_char(id);
    let cell = &mut grid[line][Column(0)];
    if !cell.zerowidth().is_some_and(|extra| extra.contains(&ch)) {
        cell.push_zerowidth(ch);
    }
}

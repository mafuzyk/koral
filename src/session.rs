use std::time::Instant;

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::Processor;

use crate::blocks::BlockStore;
use crate::graphics::{Graphics, Request};
use crate::marks::{Piece, Scanner};
use crate::view::{anchor_char, picture_anchor, picture_char, picture_foot};

/// One terminal: the grid plus the commands it has shown.
pub struct Session<L> {
    pub term: Term<L>,
    pub blocks: BlockStore,
    pub graphics: Graphics,
    /// Answers for the program on the other side of the PTY.
    pub replies: Vec<String>,
    parser: Processor,
    scanner: Scanner,
}

impl<L: EventListener> Session<L> {
    pub fn new(term: Term<L>) -> Self {
        Self {
            term,
            blocks: BlockStore::default(),
            graphics: Graphics::default(),
            replies: Vec::new(),
            parser: Processor::new(),
            scanner: Scanner::default(),
        }
    }

    pub fn feed(&mut self, bytes: &[u8], now: Instant) {
        let Self { term, blocks, graphics, replies, parser, scanner } = self;
        scanner.feed(bytes, &mut |piece| match piece {
            Piece::Bytes(bytes) => {
                parser.advance(term, bytes);
                blocks.observe(bytes);
            }
            Piece::Graphics(body) => {
                let outcome = graphics.handle(body);
                replies.extend(outcome.reply);
                if let Some(request) = outcome.place {
                    place(term, parser, graphics, request);
                }
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

/// An image rides on its top-left cell the way a block rides on its first line.
fn place<L: EventListener>(term: &mut Term<L>, parser: &mut Processor, graphics: &mut Graphics, request: Request) {
    let point = term.grid().cursor.point;
    let here: Vec<u32> = term.grid()[point.line][point.column]
        .zerowidth()
        .map_or(Vec::new(), |extra| extra.iter().filter_map(|ch| picture_anchor(*ch)).collect());
    let anchor = graphics.place(request, &here);
    if !here.contains(&anchor) {
        term.grid_mut()[point.line][point.column].push_zerowidth(picture_char(anchor));
    }

    let last = if request.keep_cursor {
        Line((point.line.0 + request.rows as i32 - 1).min(term.screen_lines() as i32 - 1))
    } else {
        let below = "\n".repeat(request.rows as usize - 1);
        parser.advance(term, format!("{below}\x1b[{}C", request.columns).as_bytes());
        term.grid().cursor.point.line
    };
    if request.rows > 1 {
        let foot = &mut term.grid_mut()[last][point.column];
        if !foot.zerowidth().is_some_and(|extra| extra.contains(&picture_foot())) {
            foot.push_zerowidth(picture_foot());
        }
    }
}

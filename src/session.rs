use std::time::Instant;

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::{Processor, Timeout};

use crate::blocks::BlockStore;
use crate::graphics::{Area, Graphics, Request};
use crate::marks::{Mark, Piece, Scanner};
use crate::notify::{BODY_LIMIT, Notice, TITLE_LIMIT, clean};
use crate::view::{anchor_char, picture_anchor, picture_char, picture_foot};

/// One terminal: the grid plus the commands it has shown.
pub struct Session<L> {
    pub term: Term<L>,
    pub blocks: BlockStore,
    pub graphics: Graphics,
    /// Answers for the program on the other side of the PTY.
    pub replies: Vec<String>,
    /// Notifications for the desktop, which also work on the alternate screen.
    pub notices: Vec<Notice>,
    parser: Processor,
    scanner: Scanner,
    /// The row the shell opened the prompt on, so the anchor cannot climb into
    /// the output written before it.
    prompt_start: Option<Line>,
    /// Columns at the last read: a change means every prompt was rewrapped.
    columns: usize,
    /// Rows at the last read: a change moves the anchor cells with the grid.
    lines: usize,
    /// Whether the program is on the alternate screen, which takes its images with it.
    alt_screen: bool,
}

impl<L: EventListener> Session<L> {
    pub fn new(term: Term<L>) -> Self {
        Self {
            alt_screen: term.mode().contains(TermMode::ALT_SCREEN),
            columns: term.grid().columns(),
            lines: term.screen_lines(),
            term,
            blocks: BlockStore::default(),
            graphics: Graphics::default(),
            replies: Vec::new(),
            notices: Vec::new(),
            parser: Processor::new(),
            scanner: Scanner::default(),
            prompt_start: None,
        }
    }

    pub fn feed(&mut self, bytes: &[u8], now: Instant) {
        let Self {
            term,
            blocks,
            graphics,
            replies,
            notices,
            parser,
            scanner,
            prompt_start,
            columns,
            lines,
            alt_screen,
        } = self;
        let (width, height) = (term.grid().columns(), term.screen_lines());
        // shortcut: the row counts are read again on the next read, so a frame drawn
        // straight after a resize can still use the wrapping the prompt had before it.
        if width != *columns || height != *lines {
            if width != *columns {
                *columns = width;
                reflow(term, blocks);
            }
            *lines = height;
            resync(term, graphics);
        }
        scanner.feed(bytes, &mut |piece| match piece {
            Piece::Bytes(bytes) => {
                advance(term, parser, graphics, bytes);
                blocks.observe(bytes);
            }
            Piece::Clear => graphics.clear_placements(),
            Piece::Graphics(body) => {
                let outcome = graphics.handle(body);
                replies.extend(outcome.reply);
                // Only a placement or a positional delete reads the grid; transfers, chunked
                // payloads, queries and id deletes settle without it and may span many reads.
                let needs_grid = outcome.place.is_some() || outcome.delete.is_some();
                // A synchronized update holds its cursor moves inside the parser, and the client
                // moved the cursor here before sending the command, so replay them first.
                // shortcut: the replay publishes what the batch drew so far, so a frame whose
                // writes span several redraws can show part of itself; only an atomic replay hides it.
                let flushed = needs_grid && parser.sync_timeout().pending_timeout();
                if flushed {
                    let (alt, history) =
                        (term.mode().contains(TermMode::ALT_SCREEN), term.grid().history_size());
                    parser.stop_sync(term);
                    follow_scroll(term, graphics, alt, history);
                    follow(term, graphics);
                }
                // The replay can leave the alternate screen: its images go before this one lands,
                // or the end-of-feed cleanup would take the image that just arrived.
                settle_alt(term, graphics, alt_screen);
                if let Some(delete) = outcome.delete {
                    let anchors = targets(term, graphics, delete.area);
                    graphics.drop(&anchors, delete.free);
                }
                if let Some(request) = outcome.place {
                    place(term, parser, graphics, request);
                }
                if flushed {
                    advance(term, parser, graphics, b"\x1b[?2026h");
                }
            }
            Piece::Status(query) => replies.push(status_reply(query, term.grid().cursor.template.flags)),
            Piece::Mark(Mark::Notify { title, body }) => {
                notices.push(Notice { title: clean(&title, TITLE_LIMIT), body: clean(&body, BODY_LIMIT) });
            }
            Piece::Mark(mark) => {
                // A program on the alternate screen must not open a block, but the
                // shell's command end arrives there too when a program dies badly.
                if term.mode().contains(TermMode::ALT_SCREEN)
                    && !matches!(mark, Mark::CommandEnd(_) | Mark::Cwd(_))
                {
                    return;
                }
                if mark == Mark::PromptStart {
                    let cursor = &term.grid().cursor;
                    // A prompt printed at the last column wraps before its first character.
                    *prompt_start = Some(Line(cursor.point.line.0 + i32::from(cursor.input_needs_wrap)));
                }
                if let Some(id) = blocks.apply(mark, now) {
                    let line = anchor(term, id, *prompt_start);
                    let feeds = blocks.get(id).map_or(0, |block| block.prompt_breaks);
                    blocks.set_prompt_lines(id, prompt_rows(term, line, feeds));
                }
            }
        });
        settle_alt(term, graphics, alt_screen);
    }
}

/// Leaving the alternate screen leaves its images behind: an app that dies must not ghost.
fn settle_alt<L: EventListener>(term: &Term<L>, graphics: &mut Graphics, alt_screen: &mut bool) {
    let alt = term.mode().contains(TermMode::ALT_SCREEN);
    if *alt_screen && !alt {
        graphics.clear_placements();
    }
    *alt_screen = alt;
}

/// Parses text, then puts back the anchors it painted over.
fn advance<L: EventListener>(term: &mut Term<L>, parser: &mut Processor, graphics: &mut Graphics, bytes: &[u8]) {
    let (alt, history) = (term.mode().contains(TermMode::ALT_SCREEN), term.grid().history_size());
    parser.advance(term, bytes);
    follow_scroll(term, graphics, alt, history);
    follow(term, graphics);
}

/// Moving the same screen's top lines into the scrollback moves every anchor up by as much.
fn follow_scroll<L: EventListener>(term: &Term<L>, graphics: &mut Graphics, alt: bool, history: usize) {
    if alt == term.mode().contains(TermMode::ALT_SCREEN) {
        graphics.scrolled(term.grid().history_size().saturating_sub(history), alt);
    }
}

/// Text may paint over the cell an image rides on, but it does not erase the image: put the anchor
/// back. A cell the grid moved the anchor away from (a scroll) is followed to its new line. Only
/// the anchors of the active screen are touched, and their column alone is walked, not every cell.
fn follow<L: EventListener>(term: &mut Term<L>, graphics: &mut Graphics) {
    if graphics.tallest() == 0 {
        return;
    }
    let alt = term.mode().contains(TermMode::ALT_SCREEN);
    let mut moves = Vec::new();
    let mut restores = Vec::new();
    {
        let grid = term.grid();
        let (top, bottom, columns) = (grid.topmost_line().0, grid.bottommost_line().0, grid.columns());
        let screen_top = bottom - (grid.screen_lines() as i32 - 1);
        // An anchor above this cannot reach the view, and text is never written off the screen.
        let window_top = (screen_top - graphics.tallest() as i32).max(top);
        for (anchor, placement) in graphics.placements() {
            if placement.alt != alt {
                continue;
            }
            let Some((line, column)) = placement.at else {
                continue;
            };
            if column >= columns {
                continue;
            }
            let ch = picture_char(anchor);
            let holds = |line: i32| {
                (window_top..=bottom).contains(&line)
                    && grid[Line(line)][Column(column)].zerowidth().is_some_and(|extra| extra.contains(&ch))
            };
            if holds(line) {
                continue;
            }
            match (window_top..=bottom).find(|line| holds(*line)) {
                Some(found) => moves.push((anchor, found, column)),
                None if (window_top..=bottom).contains(&line) => restores.push((anchor, line, column)),
                None => {}
            }
        }
    }
    for (anchor, line, column) in moves {
        graphics.locate(anchor, line, column);
    }
    // shortcut: an image scrolled out of a full scrollback in the same read that painted over it
    // is put back on its old cell; a targeted column walk cannot tell the two apart.
    for (anchor, line, column) in restores {
        let grid = term.grid_mut();
        if column >= grid.columns() || !(grid.topmost_line().0..=grid.bottommost_line().0).contains(&line) {
            continue;
        }
        let cell = &mut grid[Line(line)][Column(column)];
        let ch = picture_char(anchor);
        if !cell.zerowidth().is_some_and(|extra| extra.contains(&ch)) {
            cell.push_zerowidth(ch);
        }
    }
}

/// A resize reflows the grid, so every anchor of the active screen is read off it again.
fn resync<L: EventListener>(term: &Term<L>, graphics: &mut Graphics) {
    if graphics.tallest() == 0 {
        return;
    }
    let alt = term.mode().contains(TermMode::ALT_SCREEN);
    let grid = term.grid();
    let mut found = Vec::new();
    for line in grid.topmost_line().0..=grid.bottommost_line().0 {
        for column in 0..grid.columns() {
            let Some(extra) = grid[Line(line)][Column(column)].zerowidth() else {
                continue;
            };
            for ch in extra {
                if let Some(anchor) = picture_anchor(*ch)
                    && graphics.placement(anchor).is_some_and(|placement| placement.alt == alt)
                {
                    found.push((anchor, line, column));
                }
            }
        }
    }
    for (anchor, line, column) in found {
        graphics.locate(anchor, line, column);
    }
}

/// DECRQSS. neovim sets undercurl and reads it back to learn that it works.
// shortcut: colours and settings other than SGR go unreported, add them if a program restores them this way.
fn status_reply(query: &[u8], flags: Flags) -> String {
    const STYLES: [(Flags, &str); 11] = [
        (Flags::BOLD, "1"),
        (Flags::DIM, "2"),
        (Flags::ITALIC, "3"),
        (Flags::UNDERLINE, "4"),
        (Flags::DOUBLE_UNDERLINE, "4:2"),
        (Flags::UNDERCURL, "4:3"),
        (Flags::DOTTED_UNDERLINE, "4:4"),
        (Flags::DASHED_UNDERLINE, "4:5"),
        (Flags::INVERSE, "7"),
        (Flags::HIDDEN, "8"),
        (Flags::STRIKEOUT, "9"),
    ];
    if query != b"m" {
        return "\x1bP0$r\x1b\\".to_string();
    }
    let styles: String =
        STYLES.iter().filter(|(flag, _)| flags.contains(*flag)).map(|(_, code)| format!(";{code}")).collect();
    format!("\x1bP1$r0{styles}m\x1b\\")
}

/// Lines have no stable identity: a character on the first cell travels with
/// it through scrollback and reflow. Writing to the cell drops the anchor, so it
/// goes in after the prompt text, and sharky's prompt redraw places it again.
/// Returns the row it landed on.
fn anchor<L>(term: &mut Term<L>, id: u32, start: Option<Line>) -> Line {
    let grid = term.grid_mut();
    let last = Column(grid.columns() - 1);
    let top = grid.topmost_line();
    // The prompt begins at `start`: a wrap above it came from the output before.
    // shortcut: a prompt that wraps while the screen scrolls keeps the row of the
    // start it recorded, so its anchor can land inside its own wraps, never above them.
    let floor = start.map_or(top, |start| start.max(top));
    let mut line = grid.cursor.point.line;
    while line > floor && grid[Line(line.0 - 1)][last].flags.contains(Flags::WRAPLINE) {
        line = Line(line.0 - 1);
    }

    let ch = anchor_char(id);
    let cell = &mut grid[line][Column(0)];
    if !cell.zerowidth().is_some_and(|extra| extra.contains(&ch)) {
        cell.push_zerowidth(ch);
    }
    line
}

/// Rows the prompt takes above `line`. A reflow changes the rows but not the line
/// feeds the prompt printed, so the walk counts those feeds back from the anchor.
fn prompt_rows<L: EventListener>(term: &Term<L>, line: Line, feeds: usize) -> usize {
    let grid = term.grid();
    let last = Column(grid.columns() - 1);
    let top = grid.topmost_line();
    let wrapped = |at: Line| at > top && grid[Line(at.0 - 1)][last].flags.contains(Flags::WRAPLINE);
    let mut at = line;
    while wrapped(at) {
        at = Line(at.0 - 1);
    }
    for _ in 0..feeds {
        if at <= top {
            break;
        }
        at = Line(at.0 - 1);
        while wrapped(at) {
            at = Line(at.0 - 1);
        }
    }
    (line.0 - at.0) as usize
}

/// A resize rewraps every prompt, so their row counts are read off the grid again.
fn reflow<L: EventListener>(term: &Term<L>, blocks: &mut BlockStore) {
    let grid = term.grid();
    let base = anchor_char(0) as u32;
    for number in grid.topmost_line().0..=grid.bottommost_line().0 {
        let line = Line(number);
        let anchored = grid[line][Column(0)].zerowidth().and_then(|extra| {
            extra.iter().filter_map(|ch| (*ch as u32).checked_sub(base)).find(|id| blocks.get(*id).is_some())
        });
        let Some((id, feeds)) = anchored.and_then(|id| Some((id, blocks.get(id)?.prompt_breaks))) else {
            continue;
        };
        blocks.set_prompt_lines(id, prompt_rows(term, line, feeds));
    }
}

/// The anchors of the placements a positional delete names. The client counts cells from the top
/// left of the screen; koral counts grid lines from the bottom, so a scrolled primary screen makes
/// this approximate, while the alternate screen where programs live matches exactly.
fn targets<L: EventListener>(term: &Term<L>, graphics: &Graphics, area: Area) -> Vec<u32> {
    let grid = term.grid();
    let (columns, lines) = (grid.columns() as i32, grid.screen_lines() as i32);
    let bottom = grid.bottommost_line().0;
    let top = bottom - (lines - 1);
    let cursor = grid.cursor.point;
    let mut anchors = Vec::new();
    for line in grid.topmost_line().0..=bottom {
        for column in 0..columns {
            let Some(extra) = grid[Line(line)][Column(column as usize)].zerowidth() else {
                continue;
            };
            for anchor in extra.iter().filter_map(|ch| picture_anchor(*ch)) {
                let Some(placement) = graphics.placement(anchor) else {
                    continue;
                };
                let (first_row, last_row) = (line - top, line - top + placement.rows as i32 - 1);
                let (first_column, last_column) = (column, column + placement.columns as i32 - 1);
                let holds = |row: i32, column: i32| {
                    (first_row..=last_row).contains(&row) && (first_column..=last_column).contains(&column)
                };
                let named = match area {
                    Area::Cursor => holds(cursor.line.0, cursor.column.0 as i32),
                    Area::Cell { column, row, z } => {
                        holds(row as i32, column as i32) && z.is_none_or(|z| z == placement.z)
                    }
                    Area::Column(column) => (first_column..=last_column).contains(&(column as i32)),
                    Area::Row(row) => (first_row..=last_row).contains(&(row as i32)),
                    Area::Z(z) => z == placement.z,
                };
                if named {
                    anchors.push(anchor);
                }
            }
        }
    }
    anchors
}

/// An image rides on its top-left cell the way a block rides on its first line.
fn place<L: EventListener>(term: &mut Term<L>, parser: &mut Processor, graphics: &mut Graphics, request: Request) {
    // The cell the cursor reached: a client that centres an image moves the cursor there itself.
    let point = term.grid().cursor.point;
    let here: Vec<u32> = term.grid()[point.line][point.column]
        .zerowidth()
        .map_or(Vec::new(), |extra| extra.iter().filter_map(|ch| picture_anchor(*ch)).collect());
    let Some(anchor) = graphics.place(request, &here) else {
        return;
    };
    if !here.contains(&anchor) {
        term.grid_mut()[point.line][point.column].push_zerowidth(picture_char(anchor));
    }
    let alt = term.mode().contains(TermMode::ALT_SCREEN);
    graphics.note_placement(anchor, &request, point.column.0, point.line.0, alt);

    let last = if request.keep_cursor {
        Line((point.line.0 + request.rows as i32 - 1).min(term.screen_lines() as i32 - 1))
    } else {
        let below = "\n".repeat(request.rows as usize - 1);
        advance(term, parser, graphics, format!("{below}\x1b[{}C", request.columns).as_bytes());
        term.grid().cursor.point.line
    };
    if request.rows > 1 {
        let foot = &mut term.grid_mut()[last][point.column];
        if !foot.zerowidth().is_some_and(|extra| extra.contains(&picture_foot())) {
            foot.push_zerowidth(picture_foot());
        }
    }
}

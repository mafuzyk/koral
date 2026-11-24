use std::time::Duration;

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Point};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::search::Match;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};

use crate::blocks::{Activity, Block, BlockStore};
use crate::boxes;
use crate::config::Config;
use crate::fonts::Coverage;
use crate::format;
use crate::graphics::Graphics;
use crate::links::Link;
use crate::render::{Cell, Clip, Picture, Pictures, Quad, Shape, Span, TextLine, rgba};
use crate::settings::{
    FIELDS, FormRow, Kind, Picker, PickerKind, Settings, SettingsView, display_value, form_rows,
    visible,
};
use crate::splits::Divider;
use crate::tabs::{Bar, CLOSE_ZONE, Rect};
use crate::theme::{Theme, mix, theme_name};
use crate::view::{Row, pinned, placed, without_anchors};

const STRIP_HEIGHT: f32 = 38.0;
const TAB_TOP: f32 = 6.0;
const TAB_RADIUS: f32 = 10.0;
const GUTTER_CELLS: f32 = 2.0;
const SEARCH_COLUMNS: f32 = 36.0;
const PANEL_INSET: f32 = 12.0;
const STRIP_SHADE: [f32; 4] = [0.0, 0.0, 0.0, 0.32];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub width: f32,
    pub height: f32,
    pub scale: f32,
    pub padding: f32,
    pub cell: Cell,
}

impl Layout {
    pub fn strip(&self) -> f32 {
        STRIP_HEIGHT * self.scale
    }

    pub fn left(&self) -> f32 {
        self.padding * self.scale
    }

    pub fn gutter(&self) -> f32 {
        GUTTER_CELLS * self.cell.width
    }

    pub fn grid_x(&self) -> f32 {
        self.left() + self.gutter()
    }

    pub fn grid_y(&self) -> f32 {
        self.strip() + self.padding * self.scale / 2.0
    }

    pub fn content_rect(&self) -> Rect {
        Rect {
            x: 0.0,
            y: self.strip(),
            width: self.width,
            height: (self.height - self.strip()).max(1.0),
        }
    }

    pub fn pane(&self) -> PaneLayout {
        PaneLayout::from_rect(self.content_rect(), *self)
    }

    pub fn columns(&self) -> usize {
        self.pane().columns()
    }

    pub fn lines(&self) -> usize {
        self.pane().lines()
    }

    /// The visible (row, column) under a window position.
    pub fn cell_at(&self, x: f32, y: f32) -> (usize, usize) {
        self.pane().cell_at(x, y)
    }

    pub fn in_gutter(&self, x: f32) -> bool {
        self.pane().in_gutter(x)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaneLayout {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub scale: f32,
    pub padding: f32,
    pub cell: Cell,
}

impl PaneLayout {
    pub fn from_rect(rect: Rect, layout: Layout) -> Self {
        Self {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
            scale: layout.scale,
            padding: layout.padding,
            cell: layout.cell,
        }
    }

    pub fn left(&self) -> f32 {
        self.x + self.padding * self.scale
    }

    pub fn gutter(&self) -> f32 {
        GUTTER_CELLS * self.cell.width
    }

    pub fn grid_x(&self) -> f32 {
        self.left() + self.gutter()
    }

    pub fn grid_y(&self) -> f32 {
        self.y + self.padding * self.scale / 2.0
    }

    pub fn columns(&self) -> usize {
        (((self.width - self.padding * self.scale * 2.0 - self.gutter()) / self.cell.width)
            as usize)
            .max(2)
    }

    pub fn lines(&self) -> usize {
        (((self.height - self.padding * self.scale * 1.5) / self.cell.height) as usize).max(1)
    }

    pub fn cell_at(&self, x: f32, y: f32) -> (usize, usize) {
        let row = ((y - self.grid_y()).max(0.0) / self.cell.height) as usize;
        let column = ((x - self.grid_x()).max(0.0) / self.cell.width) as usize;
        (row.min(self.lines() - 1), column.min(self.columns() - 1))
    }

    pub fn in_gutter(&self, x: f32) -> bool {
        x >= self.x && x < self.grid_x()
    }

    pub fn clip_rect(&self) -> [f32; 4] {
        [self.x, self.y, self.width, self.height]
    }

    pub fn grid_clip_rect(&self) -> [f32; 4] {
        [
            self.grid_x(),
            self.grid_y(),
            self.columns() as f32 * self.cell.width,
            self.lines() as f32 * self.cell.height,
        ]
    }
}

#[derive(Default)]
pub struct Frame {
    pub quads: Vec<Quad>,
    pub lines: Vec<TextLine>,
    pub pictures: Pictures,
    pub clip: Clip,
}

impl Frame {
    /// Clears this frame's output while keeping every allocation for the next redraw.
    pub fn reset(&mut self) {
        self.quads.clear();
        self.lines.clear();
        self.pictures.list.clear();
        self.clip = Clip::default();
    }
}

pub struct TabView {
    pub title: String,
    pub activity: Activity,
    pub confirming: bool,
    pub rang: bool,
}

pub struct Chrome<'a> {
    pub tabs: &'a [TabView],
    pub active: usize,
    pub hovered: Option<usize>,
    pub hovered_close: bool,
    pub hovered_new: bool,
    pub hovered_settings: bool,
    pub settings_open: bool,
    pub bar: &'a Bar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorView {
    pub focused: bool,
    pub visible: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Underline {
    Single,
    Double,
    Curl,
    Dotted,
    Dashed,
}

fn underline_style(flags: Flags) -> Option<Underline> {
    [
        (Flags::UNDERCURL, Underline::Curl),
        (Flags::DOUBLE_UNDERLINE, Underline::Double),
        (Flags::DOTTED_UNDERLINE, Underline::Dotted),
        (Flags::DASHED_UNDERLINE, Underline::Dashed),
        (Flags::UNDERLINE, Underline::Single),
    ]
    .into_iter()
    .find_map(|(flag, style)| flags.contains(flag).then_some(style))
}

/// Decoration stroke width, and how far above the cell bottom the line sits.
/// Both follow the cell so an underline stays visible as the font grows.
pub fn decoration(cell: Cell) -> (f32, f32) {
    let thickness = (cell.height * 0.08).max(1.0).round();
    (thickness, (cell.height * 0.12).max(2.0 * thickness))
}

/// The strike line crosses the middle of the cell.
pub fn strike(cell: Cell) -> f32 {
    cell.height / 2.0
}

/// Drops the font a substitution notice replaced, so the reason fits one line.
/// Other errors stay whole: the part after a full stop may be the point.
pub fn brief(message: &str) -> &str {
    message
        .find(". Showing ")
        .map_or(message, |end| &message[..=end])
}

/// The marks of one cell. Dots and curls follow the window position, so neighbours join.
pub fn underline(
    style: Underline,
    x: f32,
    y: f32,
    width: f32,
    thickness: f32,
    color: [f32; 4],
) -> Vec<Quad> {
    let beats = |period: f32, mark: f32| -> Vec<Quad> {
        let end = x + width;
        let mut start = (x / period).floor() * period;
        let mut quads = Vec::new();
        while start < end {
            let (left, right) = (start.max(x), (start + mark).min(end));
            if right > left {
                quads.push(Quad::new(left, y, right - left, thickness, color));
            }
            start += period;
        }
        quads
    };
    match style {
        Underline::Single => vec![Quad::new(x, y, width, thickness, color)],
        Underline::Double => {
            vec![
                Quad::new(x, y, width, thickness, color),
                Quad::new(x, y + 2.0 * thickness, width, thickness, color),
            ]
        }
        Underline::Dotted => beats(2.0 * thickness, thickness),
        Underline::Dashed => beats(6.0 * thickness, 4.0 * thickness),
        Underline::Curl => {
            let period = 8.0 * thickness;
            let wave = |column: f32| (column * std::f32::consts::TAU / period).sin() * thickness;
            let mut quads = Vec::new();
            let mut left = x;
            while left < x + width {
                let right = (left.floor() + 1.0).min(x + width);
                quads.push(Quad::new(
                    left,
                    y + wave(left.floor()).round(),
                    right - left,
                    thickness,
                    color,
                ));
                left = right;
            }
            quads
        }
    }
}

pub fn visible_top(rows: &[Row], lines: usize, scroll: usize) -> usize {
    rows.len().saturating_sub(lines + scroll)
}

#[allow(clippy::too_many_arguments)]
pub fn draw_terminal<L: EventListener>(
    frame: &mut Frame,
    term: &Term<L>,
    blocks: &BlockStore,
    rows: &[Row],
    scroll: usize,
    cursor: CursorView,
    layout: Layout,
    theme: &Theme,
) {
    draw_terminal_in(
        frame,
        term,
        blocks,
        rows,
        scroll,
        cursor,
        layout.pane(),
        Coverage::default(),
        theme,
    );
}

#[allow(clippy::too_many_arguments)]
pub fn draw_terminal_in<L: EventListener>(
    frame: &mut Frame,
    term: &Term<L>,
    blocks: &BlockStore,
    rows: &[Row],
    scroll: usize,
    cursor: CursorView,
    pane: PaneLayout,
    coverage: Coverage,
    theme: &Theme,
) {
    draw_pane_base(
        frame, term, blocks, rows, scroll, cursor, pane, coverage, theme,
    );
    frame.pictures.over = frame.quads.len();
    draw_pane_overlays(frame, term, blocks, rows, scroll, cursor, pane, theme);
}

fn find_caret<L: EventListener>(
    term: &Term<L>,
    rows: &[Row],
    top: usize,
    scroll: usize,
    cursor: CursorView,
    pane: PaneLayout,
) -> Option<(usize, usize, f32)> {
    let grid = term.grid();
    (scroll == 0 && cursor.visible && term.mode().contains(TermMode::SHOW_CURSOR))
        .then_some(grid.cursor.point)
        .and_then(|point| {
            rows[top..]
                .iter()
                .take(pane.lines())
                .position(|row| row.line == point.line)
                .map(|index| {
                    let flags = grid[point].flags;
                    // A wide glyph takes two cells: the cursor covers both from the leading one.
                    let (column, wide) = if flags.contains(Flags::WIDE_CHAR_SPACER) {
                        (point.column.0.saturating_sub(1), 2.0)
                    } else if flags.contains(Flags::WIDE_CHAR) {
                        (point.column.0, 2.0)
                    } else {
                        (point.column.0, 1.0)
                    };
                    (index, column, wide)
                })
        })
}

#[allow(clippy::too_many_arguments)]
pub fn draw_pane_base<L: EventListener>(
    frame: &mut Frame,
    term: &Term<L>,
    blocks: &BlockStore,
    rows: &[Row],
    scroll: usize,
    cursor: CursorView,
    pane: PaneLayout,
    coverage: Coverage,
    theme: &Theme,
) {
    let top = visible_top(rows, pane.lines(), scroll);
    let selection = term
        .selection
        .as_ref()
        .and_then(|selection| selection.to_range(term));
    let grid = term.grid();
    let columns = grid.columns();
    let colors = term.colors();
    let default_bg = theme.resolve(Color::Named(NamedColor::Background), colors);
    let (thickness, offset) = decoration(pane.cell);
    // Only the focused solid block hides the glyph under it; unfocused splits draw an outline.
    let solid = cursor.focused && matches!(term.cursor_style().shape, CursorShape::Block);
    let caret = find_caret(term, rows, top, scroll, cursor, pane);
    let clip = Some(pane.clip_rect());

    for (index, row) in rows[top..].iter().take(pane.lines()).enumerate() {
        let y = pane.grid_y() + index as f32 * pane.cell.height;
        let cells = &grid[row.line];
        let mut pieces: Vec<(usize, Vec<Span>)> = Vec::new();
        let mut after_symbol = false;

        for column in 0..columns {
            let cell = &cells[Column(column)];
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }

            let mut fg = theme.resolve(cell.fg, colors);
            let mut bg = theme.resolve(cell.bg, colors);
            if cell.flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            if cell.flags.contains(Flags::DIM) {
                fg = Rgb {
                    r: fg.r / 3 * 2,
                    g: fg.g / 3 * 2,
                    b: fg.b / 3 * 2,
                };
            }
            if cell.flags.contains(Flags::HIDDEN) {
                fg = bg;
            }
            if solid
                && let Some((caret_row, caret_column, _)) = caret
                && caret_row == index
                && caret_column == column
            {
                fg = bg;
            }

            let x = pane.grid_x() + column as f32 * pane.cell.width;
            let wide = if cell.flags.contains(Flags::WIDE_CHAR) {
                2.0
            } else {
                1.0
            };
            let width = pane.cell.width * wide;
            let selected =
                selection.is_some_and(|range| range.contains(Point::new(row.line, Column(column))));
            if selected {
                frame.quads.push(Quad::new(
                    x,
                    y,
                    width,
                    pane.cell.height,
                    rgba(theme.accent, 0.35),
                ));
            } else if bg != default_bg {
                frame
                    .quads
                    .push(Quad::new(x, y, width, pane.cell.height, rgba(bg, 1.0)));
            }
            if let Some(style) = underline_style(cell.flags) {
                let color = cell
                    .underline_color()
                    .map_or(fg, |color| theme.resolve(color, colors));
                let top = y + pane.cell.height - offset;
                frame
                    .quads
                    .extend(underline(style, x, top, width, thickness, rgba(color, 1.0)));
            }
            if cell.flags.contains(Flags::STRIKEOUT) {
                frame.quads.push(Quad::new(
                    x,
                    y + strike(pane.cell),
                    width,
                    thickness,
                    rgba(fg, 1.0),
                ));
            }
            let box_thickness = pane.scale.max(1.0);

            let mut text = String::from(shown(cell.c));
            if cell.c != crate::view::PLACEHOLDER_CHAR
                && let Some(extra) = cell.zerowidth()
            {
                text.extend(extra.iter());
            }
            let mut text = without_anchors(&text);
            if let Some(quads) = boxes::quads(
                cell.c,
                x,
                y,
                width,
                pane.cell.height,
                box_thickness,
                rgba(fg, 1.0),
            ) {
                frame.quads.extend(quads);
                text = " ".to_string();
            }
            let bold = cell.flags.intersects(Flags::BOLD);
            let italic = cell.flags.intersects(Flags::ITALIC);
            let color = (fg.r, fg.g, fg.b);
            let symbol = text.chars().next().is_some_and(|ch| !coverage.matches(ch));
            if symbol || after_symbol || pieces.is_empty() {
                pieces.push((column, Vec::new()));
            }
            after_symbol = symbol;
            let spans = pieces
                .last_mut()
                .map(|(_, spans)| spans)
                .expect("a piece was just pushed");
            match spans.last_mut() {
                Some(last) if last.color == color && last.bold == bold && last.italic == italic => {
                    last.text.push_str(&text);
                }
                _ => spans.push(Span {
                    text,
                    color,
                    bold,
                    italic,
                }),
            }
        }

        for (column, mut spans) in pieces {
            while spans
                .last()
                .is_some_and(|span| span.text.trim_end().is_empty())
            {
                spans.pop();
            }
            if let Some(last) = spans.last_mut() {
                last.text.truncate(last.text.trim_end().len());
                let x = pane.grid_x() + column as f32 * pane.cell.width;
                frame.lines.push(TextLine {
                    x,
                    y,
                    small: false,
                    spans,
                    clip,
                });
            }
        }

        if let Some(block) = row.block.and_then(|id| blocks.get(id)) {
            draw_block_marks(frame, block, row, y, blocks, pane, theme);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn draw_pane_overlays<L: EventListener>(
    frame: &mut Frame,
    term: &Term<L>,
    blocks: &BlockStore,
    rows: &[Row],
    scroll: usize,
    cursor: CursorView,
    pane: PaneLayout,
    theme: &Theme,
) {
    let top = visible_top(rows, pane.lines(), scroll);
    if let Some((index, column, wide)) = find_caret(term, rows, top, scroll, cursor, pane) {
        let shape = term.cursor_style().shape;
        draw_cursor(
            frame,
            shape,
            index,
            column,
            pane.cell.width * wide,
            cursor.focused,
            pane,
            theme,
        );
    }

    if let Some(block) = pinned(rows, top, blocks).and_then(|id| blocks.get(id)) {
        draw_pinned(frame, block, pane, theme);
    }

    if scroll > 0 && rows.len() > pane.lines() {
        let track_y = pane.grid_y();
        let track_h = pane.lines() as f32 * pane.cell.height;
        let bar_w = (2.0 * pane.scale).round().max(1.0);
        let bar_x = pane.x + pane.width - bar_w - (2.0 * pane.scale).round();
        let total = rows.len() as f32;
        let max_top = (rows.len() - pane.lines()) as f32;
        let thumb_h = (pane.lines() as f32 / total * track_h)
            .max(8.0 * pane.scale)
            .min(track_h);
        let thumb_y = track_y + (top as f32 / max_top) * (track_h - thumb_h);
        frame.quads.push(Quad::new(
            bar_x,
            thumb_y,
            bar_w,
            thumb_h,
            rgba(theme.muted, 0.45),
        ));
        for (idx, row) in rows.iter().enumerate() {
            if row
                .block
                .and_then(|id| blocks.get(id))
                .and_then(|block| block.status)
                .is_some_and(|status| status != 0)
            {
                let tick_y = track_y + (idx as f32 / total) * (track_h - bar_w);
                frame.quads.push(Quad::new(
                    bar_x,
                    tick_y,
                    bar_w,
                    bar_w,
                    rgba(theme.failure, 0.85),
                ));
            }
        }
    }
}

pub fn draw_pane_dividers(
    frame: &mut Frame,
    dividers: &[Divider],
    active_rect: Option<Rect>,
    scale: f32,
    theme: &Theme,
) {
    if dividers.is_empty() {
        return;
    }
    let line = rgba(mix(theme.background, theme.foreground, 0.18), 1.0);
    for div in dividers {
        frame.quads.push(Quad::new(
            div.rect.x,
            div.rect.y,
            div.rect.width,
            div.rect.height,
            line,
        ));
    }
    if let Some(active) = active_rect {
        let bar_h = (2.0 * scale).round().max(1.0);
        frame.quads.push(Quad::new(
            active.x,
            active.y,
            active.width,
            bar_h,
            rgba(theme.accent, 0.8),
        ));
    }
}

/// The character a cell draws. alacritty keeps tabs in the grid for copying, and a font would widen them.
pub fn shown(ch: char) -> char {
    if matches!(ch, '\0' | '\t' | '\u{10EEEE}') {
        ' '
    } else {
        ch
    }
}

pub fn draw_search(
    frame: &mut Frame,
    rows: &[Row],
    top: usize,
    matches: &[Match],
    current: Option<&Match>,
    layout: Layout,
    theme: &Theme,
) {
    draw_search_in(frame, rows, top, matches, current, layout.pane(), theme);
}

pub fn draw_search_in(
    frame: &mut Frame,
    rows: &[Row],
    top: usize,
    matches: &[Match],
    current: Option<&Match>,
    pane: PaneLayout,
    theme: &Theme,
) {
    let cell = pane.cell;
    for found in matches {
        let (start, end) = (found.start(), found.end());
        for (index, row) in rows.iter().enumerate().skip(top).take(pane.lines()) {
            if row.line < start.line || row.line > end.line {
                continue;
            }
            let first = if row.line == start.line {
                start.column.0
            } else {
                0
            };
            let last = if row.line == end.line {
                end.column.0 + 1
            } else {
                pane.columns()
            };
            let (x, y) = (
                pane.grid_x() + first as f32 * cell.width,
                pane.grid_y() + (index - top) as f32 * cell.height,
            );
            let width = last.saturating_sub(first) as f32 * cell.width;
            let focused = current == Some(found);
            let alpha = if focused { 0.6 } else { 0.25 };
            frame.quads.push(Quad::new(
                x,
                y,
                width,
                cell.height,
                rgba(theme.accent, alpha),
            ));
            if focused {
                let line = pane.scale.max(1.0);
                let ink = rgba(theme.foreground, 1.0);
                frame.quads.push(Quad::new(x, y, width, line, ink));
                frame
                    .quads
                    .push(Quad::new(x, y + cell.height - line, width, line, ink));
                frame.quads.push(Quad::new(x, y, line, cell.height, ink));
                frame
                    .quads
                    .push(Quad::new(x + width - line, y, line, cell.height, ink));
            }
        }
    }
}

/// Full-width consent banner above the content, independent of tab-title width.
pub fn draw_consent(frame: &mut Frame, prompt: &str, layout: Layout, theme: &Theme) {
    let (scale, cell) = (layout.scale, layout.cell);
    let height = cell.height + 10.0 * scale;
    let y = layout.strip();
    let inset = PANEL_INSET * scale;
    frame.quads.push(Quad {
        x: 0.0,
        y,
        width: layout.width,
        height,
        color: rgba(mix(theme.background, theme.accent, 0.2), 1.0),
        shape: Shape::Rounded {
            radius: 8.0 * scale,
        },
    });
    let text = format!("{prompt} Ctrl+Shift+Enter allows, Esc denies.");
    let room = (((layout.width - inset * 2.0) / cell.width) as usize).max(1);
    frame.lines.push(TextLine {
        x: inset,
        y: y + (height - cell.height) / 2.0,
        small: false,
        spans: vec![Span::plain(fit(&text, room), theme.foreground)],
        clip: None,
    });
}

/// A panel in the bottom right corner. Gives its x, width and the y of its text.
fn bottom_panel(frame: &mut Frame, columns: f32, layout: Layout, theme: &Theme) -> (f32, f32, f32) {
    let scale = layout.scale;
    let cell = layout.cell;
    let width =
        (columns * cell.width + PANEL_INSET * scale * 2.0).min(layout.width - layout.left() * 2.0);
    let height = cell.height + 12.0 * scale;
    let (x, y) = (
        layout.width - layout.left() - width,
        layout.height - layout.left() - height,
    );
    frame.clip = Clip {
        count: frame.lines.len(),
        x,
        top: y - cell.height,
        bottom: y + height,
    };
    frame.quads.push(Quad {
        x,
        y,
        width,
        height,
        color: rgba(mix(theme.background, theme.foreground, 0.08), 1.0),
        shape: Shape::Rounded {
            radius: 10.0 * scale,
        },
    });
    (x, width, y + (height - cell.height) / 2.0)
}

pub fn draw_search_bar(frame: &mut Frame, query: &str, found: bool, layout: Layout, theme: &Theme) {
    draw_search_bar_status(frame, query, found, None, layout, theme);
}

pub fn draw_search_bar_status(
    frame: &mut Frame,
    query: &str,
    found: bool,
    position: Option<(usize, usize)>,
    layout: Layout,
    theme: &Theme,
) {
    let (scale, cell) = (layout.scale, layout.cell);
    let inset = PANEL_INSET * scale;
    let (x, width, text_y) = bottom_panel(frame, SEARCH_COLUMNS, layout, theme);
    let (status, status_color) = if query.is_empty() {
        (String::new(), theme.muted)
    } else if !found {
        ("No matches".to_string(), theme.failure)
    } else if let Some((index, total)) = position {
        (format!("{index}/{total}"), theme.muted)
    } else {
        (String::new(), theme.muted)
    };
    let room =
        (((width - inset * 2.0) / cell.width) as usize).saturating_sub(status.chars().count());
    let shown = fit(query, room.saturating_sub(1));
    let caret = x + inset + shown.chars().count() as f32 * cell.width;
    frame.quads.push(Quad::new(
        caret,
        text_y,
        (2.0 * scale).round(),
        cell.height,
        rgba(theme.cursor, 0.9),
    ));
    let spans = if query.is_empty() {
        vec![Span::plain("Search", theme.muted)]
    } else {
        vec![Span::plain(shown, theme.foreground)]
    };
    frame.lines.push(TextLine {
        x: x + inset,
        y: text_y,
        small: false,
        spans,
        clip: None,
    });
    if !status.is_empty() {
        let status_x = x + width - inset - status.chars().count() as f32 * cell.width;
        frame.lines.push(TextLine {
            x: status_x,
            y: text_y,
            small: false,
            spans: vec![Span::plain(status, status_color)],
            clip: None,
        });
    }
}

pub fn draw_link(
    frame: &mut Frame,
    rows: &[Row],
    top: usize,
    link: &Link,
    layout: Layout,
    theme: &Theme,
) {
    draw_link_in(frame, rows, top, link, layout.pane(), theme);
}

pub fn draw_link_in(
    frame: &mut Frame,
    rows: &[Row],
    top: usize,
    link: &Link,
    pane: PaneLayout,
    theme: &Theme,
) {
    let (start, end) = (link.range.start(), link.range.end());
    let cell = pane.cell;
    let line = pane.scale.max(1.0);
    for (index, row) in rows.iter().enumerate().skip(top).take(pane.lines()) {
        if row.line < start.line || row.line > end.line {
            continue;
        }
        let first = if row.line == start.line {
            start.column.0
        } else {
            0
        };
        let last = if row.line == end.line {
            end.column.0 + 1
        } else {
            pane.columns()
        };
        let x = pane.grid_x() + first as f32 * cell.width;
        let y = pane.grid_y() + (index - top + 1) as f32 * cell.height - 2.0 * line;
        frame.quads.push(Quad::new(
            x,
            y,
            last.saturating_sub(first) as f32 * cell.width,
            line,
            rgba(theme.accent, 1.0),
        ));
    }
}

/// The real target, since the text on screen can say anything.
pub fn draw_link_bar(frame: &mut Frame, uri: &str, layout: Layout, theme: &Theme) {
    let cell = layout.cell;
    let inset = PANEL_INSET * layout.scale;
    let (x, width, y) = bottom_panel(frame, uri.chars().count() as f32, layout, theme);
    let shown = fit(uri, ((width - inset * 2.0) / cell.width) as usize);
    frame.lines.push(TextLine {
        x: x + inset,
        y,
        small: false,
        spans: vec![Span::plain(shown, theme.foreground)],
        clip: None,
    });
}

/// The size a placement draws at, and the part of the image it shows. The cells are the size: the
/// whole source crop is mapped into the cell rectangle, stretched to it as kitty does
/// (`graphics.c` `grman_put`, which builds the dest rect from the cells alone).
pub fn picture_box(image: (u32, u32), crop: [u32; 4], area: (f32, f32)) -> ((f32, f32), [f32; 4]) {
    let [x, y, width, height] = crop;
    let (whole_width, whole_height) = (image.0 as f32, image.1 as f32);
    (
        area,
        [
            x as f32 / whole_width,
            y as f32 / whole_height,
            width as f32 / whole_width,
            height as f32 / whole_height,
        ],
    )
}

pub fn draw_pictures<L: EventListener>(
    frame: &mut Frame,
    term: &Term<L>,
    graphics: &Graphics,
    rows: &[Row],
    scroll: usize,
    layout: Layout,
) {
    draw_pictures_in(frame, term, graphics, rows, scroll, layout.pane());
}

pub fn draw_pictures_in<L: EventListener>(
    frame: &mut Frame,
    term: &Term<L>,
    graphics: &Graphics,
    rows: &[Row],
    scroll: usize,
    pane: PaneLayout,
) {
    let cell = pane.cell;
    let top = visible_top(rows, pane.lines(), scroll);
    let area_rect = pane.grid_clip_rect();
    for found in placed(term, graphics, rows, top, pane.lines()) {
        let Some(image) = graphics.image(found.image) else {
            continue;
        };
        let area = (
            found.columns as f32 * cell.width,
            found.rows as f32 * cell.height,
        );
        let ((width, height), crop) = picture_box((image.width, image.height), found.crop, area);
        let x = pane.grid_x() + found.column as f32 * cell.width;
        let y = pane.grid_y() + found.row as f32 * cell.height;
        let picture = Picture {
            image: image.clone(),
            id: found.image,
            number: found.number,
            z: found.z,
            x,
            y,
            width,
            height,
            crop,
            clip: Some(area_rect),
        };
        frame.pictures.list.push(picture);
    }
    frame.pictures.area = area_rect;
}

fn draw_block_marks(
    frame: &mut Frame,
    block: &Block,
    row: &Row,
    y: f32,
    blocks: &BlockStore,
    pane: PaneLayout,
    theme: &Theme,
) {
    if block.status.is_none() && block.command.is_none() {
        return;
    }
    let clip = Some(pane.clip_rect());
    let toggle = if block.folded { "▸" } else { "▾" };
    let toggle_color = match block.status {
        Some(0) => theme.success,
        Some(_) => theme.failure,
        None => theme.accent,
    };
    frame.lines.push(TextLine {
        x: pane.left(),
        y,
        small: false,
        spans: vec![Span::plain(toggle, toggle_color)],
        clip,
    });

    let running = blocks.activity() == Activity::Running
        && blocks.latest().is_some_and(|last| last.id == block.id);
    let mut spans = Vec::new();
    if row.hidden > 0 {
        spans.push(Span::plain(
            format!("{}  ", format::lines(row.hidden)),
            theme.muted,
        ));
    }
    match block.status {
        Some(0) => spans.push(Span::plain("✓", theme.success)),
        Some(status) => spans.push(Span::plain(format!("✗ {status}"), theme.failure)),
        None if running => spans.push(Span::plain("running", theme.accent)),
        None => {}
    }
    if let Some(duration) = block
        .duration
        .filter(|value| *value >= Duration::from_millis(100))
    {
        spans.push(Span::plain(
            format!("  {}", format::duration(duration)),
            theme.muted,
        ));
    }
    if spans.is_empty() {
        return;
    }

    let characters: usize = spans.iter().map(|span| span.text.chars().count()).sum();
    let x = pane.x + pane.width
        - pane.padding * pane.scale
        - characters as f32 * pane.cell.width * 0.82;
    frame.lines.push(TextLine {
        x,
        y,
        small: true,
        spans,
        clip,
    });
}

/// Where the cursor sits on screen, when it is on screen.
pub fn cursor_position<L: EventListener>(
    term: &Term<L>,
    rows: &[Row],
    scroll: usize,
    layout: Layout,
) -> Option<(f32, f32)> {
    cursor_position_in(term, rows, scroll, layout.pane())
}

pub fn cursor_position_in<L: EventListener>(
    term: &Term<L>,
    rows: &[Row],
    scroll: usize,
    pane: PaneLayout,
) -> Option<(f32, f32)> {
    let top = visible_top(rows, pane.lines(), scroll);
    let cursor = term.grid().cursor.point;
    let index = rows[top..]
        .iter()
        .take(pane.lines())
        .position(|row| row.line == cursor.line)?;
    let x = pane.grid_x() + cursor.column.0 as f32 * pane.cell.width;
    Some((x, pane.grid_y() + index as f32 * pane.cell.height))
}

/// Text an input method is still composing.
// shortcut: counts characters as one cell each, wide scripts underline short.
pub fn draw_preedit(frame: &mut Frame, text: &str, x: f32, y: f32, layout: Layout, theme: &Theme) {
    let width = text.chars().count() as f32 * layout.cell.width;
    let (thickness, offset) = decoration(layout.cell);
    frame.quads.push(Quad::new(
        x,
        y,
        width,
        layout.cell.height,
        rgba(theme.background, 1.0),
    ));
    frame.quads.push(Quad::new(
        x,
        y + layout.cell.height - offset,
        width,
        thickness,
        rgba(theme.accent, 1.0),
    ));
    frame.lines.push(TextLine {
        x,
        y,
        small: false,
        spans: vec![Span::plain(text.to_string(), theme.foreground)],
        clip: None,
    });
}

#[allow(clippy::too_many_arguments)]
fn draw_cursor(
    frame: &mut Frame,
    shape: CursorShape,
    index: usize,
    column: usize,
    width: f32,
    focused: bool,
    pane: PaneLayout,
    theme: &Theme,
) {
    let x = pane.grid_x() + column as f32 * pane.cell.width;
    let y = pane.grid_y() + index as f32 * pane.cell.height;
    let thickness = (2.0 * pane.scale).round().max(1.0);
    let color = rgba(theme.cursor, if focused { 0.9 } else { 0.5 });
    let height = pane.cell.height;
    if !focused && matches!(shape, CursorShape::Block) {
        let edge = pane.scale.max(1.0);
        frame.quads.push(Quad::new(x, y, width, edge, color));
        frame
            .quads
            .push(Quad::new(x, y + height - edge, width, edge, color));
        frame.quads.push(Quad::new(x, y, edge, height, color));
        frame
            .quads
            .push(Quad::new(x + width - edge, y, edge, height, color));
        return;
    }
    frame.quads.push(match shape {
        CursorShape::Beam => Quad::new(x, y, thickness, height, color),
        CursorShape::Underline => Quad::new(x, y + height - thickness, width, thickness, color),
        CursorShape::Hidden => return,
        _ => Quad::new(x, y, width, height, color),
    });
}

fn draw_pinned(frame: &mut Frame, block: &Block, pane: PaneLayout, theme: &Theme) {
    let y = pane.grid_y();
    let clip = Some(pane.clip_rect());
    frame.quads.push(Quad::new(
        pane.x,
        y,
        pane.width,
        pane.cell.height,
        rgba(mix(theme.background, theme.foreground, 0.08), 1.0),
    ));
    let command = block
        .command
        .as_deref()
        .and_then(|text| text.lines().next())
        .unwrap_or("running");
    frame.lines.push(TextLine {
        x: pane.left(),
        y,
        small: false,
        spans: vec![Span::plain(format!("▾ {command}"), theme.foreground)],
        clip,
    });
}

pub fn draw_settings(
    frame: &mut Frame,
    settings: &Settings,
    config: &Config,
    error: Option<&str>,
    layout: Layout,
    theme: &Theme,
) {
    let view = SettingsView::new(
        layout.width,
        layout.height,
        layout.strip(),
        layout.scale,
        layout.cell.height,
    );
    let scale = layout.scale;
    let small_cell = layout.cell.width * 0.82;
    let inset = view.inset();

    // Header
    let header_clip = Some([
        view.container.x,
        view.header_y,
        view.container.width,
        view.row_height,
    ]);
    frame.lines.push(TextLine {
        x: view.container.x,
        y: view.header_y,
        small: false,
        spans: vec![Span::plain("Settings", theme.foreground)],
        clip: header_clip,
    });
    let subtitle = "~/.config/koral/koral.conf";
    let sub_x = (view.container.x + view.container.width
        - subtitle.chars().count() as f32 * small_cell)
        .max(view.container.x + 120.0 * scale);
    frame.lines.push(TextLine {
        x: sub_x,
        y: view.header_y + 2.0 * scale,
        small: true,
        spans: vec![Span::plain(subtitle, theme.muted)],
        clip: header_clip,
    });
    frame.quads.push(Quad::new(
        view.container.x,
        view.list_y - 4.0 * scale,
        view.container.width,
        scale.max(1.0),
        rgba(theme.muted, 0.25),
    ));

    // Left panel container
    frame.quads.push(Quad {
        x: view.left.x,
        y: view.left.y,
        width: view.left.width,
        height: view.left.height,
        color: rgba(mix(theme.background, theme.foreground, 0.04), 1.0),
        shape: Shape::Rounded {
            radius: 10.0 * scale,
        },
    });

    if let Some(picker) = &settings.picker {
        draw_picker(frame, settings, picker, config, view, layout, theme);
    } else {
        draw_form(frame, settings, config, view, layout, theme);
    }

    if let Some(right) = view.right {
        draw_preview_column(frame, settings, config, view, right, layout, theme);
    }

    // Hint / error footer
    let kind = FIELDS[settings.selected].kind;
    let (hint, color) = match (
        error,
        settings.editing.is_some(),
        settings.picker.as_ref().map(|p| p.kind),
    ) {
        (Some(message), _, _) => (brief(message), theme.failure),
        (None, true, _) => ("Type value. Enter saves, Esc cancels.", theme.muted),
        (None, false, Some(PickerKind::Theme)) => (
            "Arrows preview theme. Type to filter. Enter saves, Esc reverts.",
            theme.muted,
        ),
        (None, false, Some(PickerKind::Font)) => (
            "Type to filter fonts. Enter applies, Esc goes back.",
            theme.muted,
        ),
        (None, false, None) if kind == Kind::Theme => (
            "Left/Right cycles presets, Enter opens picker.",
            theme.muted,
        ),
        (None, false, None) if kind == Kind::Font => {
            ("Enter lists installed monospace fonts.", theme.muted)
        }
        (None, false, None) if kind == Kind::Toggle => {
            ("Enter or arrows toggle this setting.", theme.muted)
        }
        (None, false, None) => (
            "Left/Right adjusts, Up/Down selects, Esc closes tab.",
            theme.muted,
        ),
    };
    let room = (view.container.width / small_cell) as usize;
    frame.lines.push(TextLine {
        x: view.container.x,
        y: view.hint_y + (view.row_height - layout.cell.height) / 2.0,
        small: true,
        spans: vec![Span::plain(fit(hint, room), color)],
        clip: Some([
            view.container.x,
            view.hint_y,
            view.container.width,
            view.row_height,
        ]),
    });
    let _ = inset;
}

fn draw_form(
    frame: &mut Frame,
    settings: &Settings,
    config: &Config,
    view: SettingsView,
    layout: Layout,
    theme: &Theme,
) {
    let scale = layout.scale;
    let inset = view.inset();
    let small_cell = layout.cell.width * 0.82;
    let left_clip = Some([view.left.x, view.left.y, view.left.width, view.left.height]);
    let rows = form_rows();
    let max_scroll = rows.len().saturating_sub(view.visible_rows);
    let scroll = settings.scroll.min(max_scroll);

    for (visual, entry) in rows.iter().skip(scroll).take(view.visible_rows).enumerate() {
        let top = view.left.y + visual as f32 * view.row_height;
        let y = top + (view.row_height - layout.cell.height) / 2.0;
        match *entry {
            FormRow::Header(section) => {
                frame.lines.push(TextLine {
                    x: view.left.x + inset,
                    y,
                    small: true,
                    spans: vec![Span::plain(section.label(), theme.accent)],
                    clip: left_clip,
                });
                frame.quads.push(Quad::new(
                    view.left.x + inset,
                    top + view.row_height - 2.0 * scale,
                    view.left.width - inset * 2.0,
                    scale.max(1.0),
                    rgba(theme.muted, 0.2),
                ));
            }
            FormRow::Field(index) => {
                let field = FIELDS[index];
                let selected = index == settings.selected;
                if selected {
                    frame
                        .quads
                        .push(selected_row(view.left, top, view.row_height, scale, theme));
                    frame.quads.push(Quad::new(
                        view.left.x + 5.0 * scale,
                        top + 5.0 * scale,
                        (3.0 * scale).round().max(2.0),
                        view.row_height - 10.0 * scale,
                        rgba(theme.accent, 1.0),
                    ));
                }
                let label = if selected {
                    theme.foreground
                } else {
                    theme.muted
                };
                frame.lines.push(TextLine {
                    x: view.left.x + inset + 4.0 * scale,
                    y,
                    small: true,
                    spans: vec![Span::plain(field.label, label)],
                    clip: left_clip,
                });

                let right = view.left.x + view.left.width - inset;
                match (field.kind, settings.editing.as_ref().filter(|_| selected)) {
                    (Kind::Number { .. } | Kind::Choice(_), _) => {
                        let text = display_value(index, config);
                        if let Some((less_x, more_x)) = view.arrows(field.kind) {
                            for (ax, arrow) in [(less_x, "‹"), (more_x, "›")] {
                                frame.lines.push(TextLine {
                                    x: ax + (view.arrow_width - small_cell) / 2.0,
                                    y,
                                    small: true,
                                    spans: vec![Span::plain(arrow, theme.accent)],
                                    clip: left_clip,
                                });
                            }
                            let val_w = text.chars().count() as f32 * small_cell;
                            let middle = (less_x + view.arrow_width + more_x) / 2.0;
                            frame.lines.push(TextLine {
                                x: middle - val_w / 2.0,
                                y,
                                small: true,
                                spans: vec![Span::plain(text, theme.foreground)],
                                clip: left_clip,
                            });
                        }
                    }
                    (Kind::Theme, _) => {
                        let text = display_value(index, config);
                        if let Some((less_x, more_x)) = view.arrows(Kind::Theme) {
                            for (ax, arrow) in [(less_x, "‹"), (more_x, "›")] {
                                frame.lines.push(TextLine {
                                    x: ax + (view.arrow_width - small_cell) / 2.0,
                                    y,
                                    small: true,
                                    spans: vec![Span::plain(arrow, theme.accent)],
                                    clip: left_clip,
                                });
                            }
                            let avail = ((more_x - (less_x + view.arrow_width) - 4.0 * scale)
                                / small_cell) as usize;
                            let shown = fit(&text, avail.max(4));
                            let val_w = shown.chars().count() as f32 * small_cell;
                            let middle = (less_x + view.arrow_width + more_x) / 2.0;
                            frame.lines.push(TextLine {
                                x: middle - val_w / 2.0,
                                y,
                                small: true,
                                spans: vec![Span::plain(shown, theme.foreground)],
                                clip: left_clip,
                            });
                        }
                    }
                    (Kind::Toggle, _) => {
                        let on = display_value(index, config) == "on";
                        let (width, height, gap) = (30.0 * scale, 16.0 * scale, 3.0 * scale);
                        let state = if on { "on" } else { "off" };
                        let state_width = state.chars().count() as f32 * small_cell;
                        frame.lines.push(TextLine {
                            x: right - width - 8.0 * scale - state_width,
                            y,
                            small: true,
                            spans: vec![Span::plain(state, theme.muted)],
                            clip: left_clip,
                        });
                        let track = top + (view.row_height - height) / 2.0;
                        let knob = height - gap * 2.0;
                        frame.quads.push(Quad {
                            x: right - width,
                            y: track,
                            width,
                            height,
                            color: if on {
                                rgba(theme.accent, 1.0)
                            } else {
                                rgba(theme.muted, 0.35)
                            },
                            shape: Shape::Rounded {
                                radius: height / 2.0,
                            },
                        });
                        frame.quads.push(Quad {
                            x: if on {
                                right - gap - knob
                            } else {
                                right - width + gap
                            },
                            y: track + gap,
                            width: knob,
                            height: knob,
                            color: rgba(
                                if on {
                                    theme.background
                                } else {
                                    theme.foreground
                                },
                                1.0,
                            ),
                            shape: Shape::Rounded { radius: knob / 2.0 },
                        });
                    }
                    (Kind::Text | Kind::Font, editing) => {
                        let suffix = if field.kind == Kind::Font {
                            "  ›"
                        } else {
                            ""
                        };
                        let room = ((view.left.width * 0.50 - inset) / small_cell) as usize;
                        let text = editing
                            .cloned()
                            .unwrap_or_else(|| display_value(index, config));
                        let shown = format!(
                            "{}{}",
                            fit(&text, room.saturating_sub(suffix.len())),
                            suffix
                        );
                        let width = shown.chars().count() as f32 * small_cell;
                        let color = if editing.is_some() {
                            theme.foreground
                        } else {
                            theme.muted
                        };
                        frame.lines.push(TextLine {
                            x: right - width,
                            y,
                            small: true,
                            spans: vec![Span::plain(shown, color)],
                            clip: left_clip,
                        });
                        if editing.is_some() {
                            let caret = Quad::new(
                                right + scale,
                                y + 3.0 * scale,
                                2.0 * scale,
                                layout.cell.height - 6.0 * scale,
                                rgba(theme.accent, 1.0),
                            );
                            frame.quads.push(caret);
                        }
                    }
                }
            }
        }
    }
}

fn selected_row(box_rect: Rect, top: f32, row_height: f32, scale: f32, theme: &Theme) -> Quad {
    Quad {
        x: box_rect.x + 4.0 * scale,
        y: top + 2.0 * scale,
        width: box_rect.width - 8.0 * scale,
        height: row_height - 4.0 * scale,
        color: rgba(theme.foreground, 0.08),
        shape: Shape::Rounded {
            radius: 7.0 * scale,
        },
    }
}

/// Filterable list replacing the form inside the left column when choosing a Font or Theme.
fn draw_picker(
    frame: &mut Frame,
    settings: &Settings,
    picker: &Picker,
    config: &Config,
    view: SettingsView,
    layout: Layout,
    theme: &Theme,
) {
    let scale = layout.scale;
    let inset = view.inset();
    let small_cell = layout.cell.width * 0.82;
    let left_clip = Some([view.left.x, view.left.y, view.left.width, view.left.height]);
    let rows_cap = view.picker_rows();

    // Filter search box on row 0
    frame.quads.push(Quad {
        x: view.left.x + 6.0 * scale,
        y: view.left.y + 3.0 * scale,
        width: view.left.width - 12.0 * scale,
        height: view.row_height - 6.0 * scale,
        color: rgba(mix(theme.background, theme.foreground, 0.09), 1.0),
        shape: Shape::Rounded {
            radius: 6.0 * scale,
        },
    });
    let search_y = view.left.y + (view.row_height - layout.cell.height) / 2.0;
    let room = ((view.left.width - inset * 2.0) / small_cell) as usize;
    let (filter_text, filter_color) = if picker.query.is_empty() {
        let placeholder = match picker.kind {
            PickerKind::Font => "Filter installed monospace fonts...",
            PickerKind::Theme => "Filter built-in themes...",
        };
        (placeholder.to_string(), theme.muted)
    } else {
        (fit(&picker.query, room.saturating_sub(1)), theme.foreground)
    };
    frame.lines.push(TextLine {
        x: view.left.x + inset,
        y: search_y,
        small: true,
        spans: vec![Span::plain(filter_text.clone(), filter_color)],
        clip: left_clip,
    });
    if !picker.query.is_empty() {
        let caret_x = view.left.x + inset + filter_text.chars().count() as f32 * small_cell + scale;
        frame.quads.push(Quad::new(
            caret_x,
            search_y + 3.0 * scale,
            2.0 * scale,
            layout.cell.height - 6.0 * scale,
            rgba(theme.accent, 1.0),
        ));
    }

    match picker.kind {
        PickerKind::Font => {
            let found = settings.found();
            if found.is_empty() {
                frame.lines.push(TextLine {
                    x: view.left.x + inset,
                    y: view.left.y + view.row_height + (view.row_height - layout.cell.height) / 2.0,
                    small: true,
                    spans: vec![Span::plain("No matching fonts", theme.failure)],
                    clip: left_clip,
                });
                return;
            }
            for (row, index) in visible(picker.selected, found.len(), rows_cap).enumerate() {
                let top = view.left.y + (row + 1) as f32 * view.row_height;
                let y = top + (view.row_height - layout.cell.height) / 2.0;
                if index == picker.selected {
                    frame
                        .quads
                        .push(selected_row(view.left, top, view.row_height, scale, theme));
                }
                let color = if found[index] == config.font {
                    theme.accent
                } else {
                    theme.foreground
                };
                frame.lines.push(TextLine {
                    x: view.left.x + inset,
                    y,
                    small: true,
                    spans: vec![Span::plain(fit(found[index], room), color)],
                    clip: left_clip,
                });
            }
        }
        PickerKind::Theme => {
            let found = settings.found_themes();
            if found.is_empty() {
                frame.lines.push(TextLine {
                    x: view.left.x + inset,
                    y: view.left.y + view.row_height + (view.row_height - layout.cell.height) / 2.0,
                    small: true,
                    spans: vec![Span::plain("No matching themes", theme.failure)],
                    clip: left_clip,
                });
                return;
            }
            let current_id = config.theme.as_deref().unwrap_or("sharky");
            let swatch = (10.0 * scale).round().max(6.0);
            let gap = 3.0 * scale;
            let swatches_w = 6.0 * swatch + 5.0 * gap;
            let text_room =
                ((view.left.width - inset * 2.0 - swatches_w - 8.0 * scale) / small_cell) as usize;
            for (row, index) in visible(picker.selected, found.len(), rows_cap).enumerate() {
                let info = found[index];
                let top = view.left.y + (row + 1) as f32 * view.row_height;
                let y = top + (view.row_height - layout.cell.height) / 2.0;
                if index == picker.selected {
                    frame
                        .quads
                        .push(selected_row(view.left, top, view.row_height, scale, theme));
                }
                let color = if info.id == current_id {
                    theme.accent
                } else {
                    theme.foreground
                };
                frame.lines.push(TextLine {
                    x: view.left.x + inset,
                    y,
                    small: true,
                    spans: vec![Span::plain(fit(info.name, text_room), color)],
                    clip: left_clip,
                });
                if let Some(preset) = Theme::builtin(info.id) {
                    let sy = top + (view.row_height - swatch) / 2.0;
                    let sx0 = view.left.x + view.left.width - inset - swatches_w;
                    let dots = [
                        preset.background,
                        preset.foreground,
                        preset.accent,
                        preset.ansi[1],
                        preset.ansi[2],
                        preset.ansi[4],
                    ];
                    for (i, c) in dots.into_iter().enumerate() {
                        frame.quads.push(Quad {
                            x: sx0 + i as f32 * (swatch + gap),
                            y: sy,
                            width: swatch,
                            height: swatch,
                            color: rgba(c, 1.0),
                            shape: Shape::Rounded {
                                radius: 2.0 * scale,
                            },
                        });
                    }
                }
            }
        }
    }
}

fn draw_preview_column(
    frame: &mut Frame,
    settings: &Settings,
    config: &Config,
    view: SettingsView,
    right: Rect,
    layout: Layout,
    theme: &Theme,
) {
    let scale = layout.scale;
    let pad = 14.0 * scale;
    let small_cell = layout.cell.width * 0.82;
    let line_h = layout.cell.height;
    let clip = Some([right.x, right.y, right.width, right.height]);

    frame.quads.push(Quad {
        x: right.x,
        y: right.y,
        width: right.width,
        height: right.height,
        color: rgba(mix(theme.background, theme.foreground, 0.04), 1.0),
        shape: Shape::Rounded {
            radius: 10.0 * scale,
        },
    });

    let active_theme_label = if config.matugen && settings.preview_theme().is_none() {
        "Matugen".to_string()
    } else {
        let id = settings
            .preview_theme()
            .or(config.theme.as_deref())
            .unwrap_or("sharky");
        theme_name(id).unwrap_or(id).to_string()
    };

    let inner_x = right.x + pad;
    let inner_w = right.width - pad * 2.0;
    frame.lines.push(TextLine {
        x: inner_x,
        y: right.y + pad * 0.75,
        small: true,
        spans: vec![
            Span::plain("Preview  ", theme.muted),
            Span::plain(
                fit(
                    &active_theme_label,
                    ((inner_w - 80.0 * scale) / small_cell) as usize,
                ),
                theme.accent,
            ),
        ],
        clip,
    });

    // Mini terminal viewport
    let term_y = right.y + pad * 0.75 + line_h + 6.0 * scale;
    let term_h = line_h * 5.0 + pad * 1.4;
    frame.quads.push(Quad {
        x: inner_x,
        y: term_y,
        width: inner_w,
        height: term_h,
        color: rgba(theme.background, 1.0),
        shape: Shape::Rounded {
            radius: 8.0 * scale,
        },
    });

    let tx = inner_x + 12.0 * scale;
    let ty0 = term_y + pad * 0.65;
    let samples: [Vec<Span>; 5] = [
        vec![
            Span::plain("▾ ", theme.success),
            Span::plain("~/projects/koral  ", theme.accent),
            Span::plain("✓ 142ms", theme.success),
        ],
        vec![
            Span::plain("$ ", theme.muted),
            Span {
                text: "cargo ".to_string(),
                color: (theme.foreground.r, theme.foreground.g, theme.foreground.b),
                bold: true,
                italic: false,
            },
            Span::plain("test --release", theme.ansi[4]),
        ],
        vec![
            Span::plain("ok ", theme.success),
            Span::plain("500 passed", theme.foreground),
            Span::plain("  warn ", theme.ansi[3]),
            Span::plain("0 new", theme.muted),
        ],
        vec![
            Span {
                text: "// italic & bold sample ".to_string(),
                color: (theme.muted.r, theme.muted.g, theme.muted.b),
                bold: false,
                italic: true,
            },
            Span::plain("ERR", theme.failure),
        ],
        vec![
            Span::plain("❯ ", theme.accent),
            Span::plain("echo ready", theme.foreground),
        ],
    ];
    for (idx, spans) in samples.into_iter().enumerate() {
        frame.lines.push(TextLine {
            x: tx,
            y: ty0 + idx as f32 * line_h,
            small: true,
            spans,
            clip,
        });
    }
    let cursor_x = tx + "❯ echo ready".chars().count() as f32 * small_cell + 2.0 * scale;
    frame.quads.push(Quad::new(
        cursor_x,
        ty0 + 4.0 * line_h + 2.0 * scale,
        small_cell,
        line_h - 4.0 * scale,
        rgba(theme.cursor, 0.9),
    ));

    // 16 ANSI colour tiles (2x8 grid) + semantic colours
    let pal_y = term_y + term_h + 12.0 * scale;
    frame.lines.push(TextLine {
        x: inner_x,
        y: pal_y,
        small: true,
        spans: vec![Span::plain("ANSI 0–15", theme.muted)],
        clip,
    });
    let gap = 5.0 * scale;
    let tile_w = ((inner_w - gap * 7.0) / 8.0).max(8.0);
    let tile_h = (16.0 * scale).round();
    let grid_y = pal_y + line_h + 4.0 * scale;
    for (idx, &ansi) in theme.ansi.iter().enumerate() {
        let col = idx % 8;
        let row = idx / 8;
        frame.quads.push(Quad {
            x: inner_x + col as f32 * (tile_w + gap),
            y: grid_y + row as f32 * (tile_h + gap),
            width: tile_w,
            height: tile_h,
            color: rgba(ansi, 1.0),
            shape: Shape::Rounded {
                radius: 4.0 * scale,
            },
        });
    }

    // Shortcuts legend if there is vertical space below palette
    let short_y = grid_y + (tile_h + gap) * 2.0 + 10.0 * scale;
    let shortcuts = [
        ("Ctrl+Shift+D / E", "Split right / down"),
        ("Ctrl+Shift+Arrows", "Focus pane"),
        ("Ctrl+Shift+F / G", "Fold / copy block"),
        ("Ctrl+Shift+S", "Search scrollback"),
    ];
    let needed = line_h * (1 + shortcuts.len()) as f32;
    if short_y + needed <= right.y + right.height - 6.0 * scale {
        frame.lines.push(TextLine {
            x: inner_x,
            y: short_y,
            small: true,
            spans: vec![Span::plain("Shortcuts", theme.muted)],
            clip,
        });
        for (i, (keys, desc)) in shortcuts.into_iter().enumerate() {
            let y = short_y + (i + 1) as f32 * line_h;
            frame.lines.push(TextLine {
                x: inner_x,
                y,
                small: true,
                spans: vec![Span::plain(keys, theme.foreground)],
                clip,
            });
            let desc_x = right.x + right.width - pad - desc.chars().count() as f32 * small_cell;
            frame.lines.push(TextLine {
                x: desc_x.max(inner_x + 140.0 * scale),
                y,
                small: true,
                spans: vec![Span::plain(desc, theme.muted)],
                clip,
            });
        }
    }
    let _ = view;
}

pub fn draw_chrome(frame: &mut Frame, chrome: &Chrome<'_>, layout: Layout, theme: &Theme) {
    let scale = layout.scale;
    let strip = layout.strip();
    let top = TAB_TOP * scale;
    let radius = TAB_RADIUS * scale;
    let active = chrome.bar.tabs.get(chrome.active).copied().unwrap_or(Rect {
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 0.0,
    });
    shade_around_tab(frame, active, layout.width, strip, top, radius);

    for (index, (tab, rect)) in chrome.tabs.iter().zip(&chrome.bar.tabs).enumerate() {
        let is_active = index == chrome.active;
        let hovered = chrome.hovered == Some(index);
        if hovered && !is_active {
            let inset = 3.0 * scale;
            frame.quads.push(Quad {
                x: rect.x + inset,
                y: top + inset,
                width: rect.width - inset * 2.0,
                height: strip - top - inset * 2.0,
                color: rgba(theme.foreground, 0.07),
                shape: Shape::Rounded {
                    radius: 6.0 * scale,
                },
            });
        }

        let y = (top + strip - layout.cell.height) / 2.0;
        let (glyph, glyph_color) = match tab.activity {
            Activity::Running => ("●", theme.accent),
            Activity::Succeeded => ("✓", theme.success),
            Activity::Failed => ("✗", theme.failure),
            Activity::Idle => ("", theme.muted),
        };
        let small_cell = layout.cell.width * 0.82;
        let mut x = rect.x + 12.0 * scale;
        if !glyph.is_empty() {
            frame.lines.push(TextLine {
                x,
                y,
                small: true,
                spans: vec![Span::plain(glyph, glyph_color)],
                clip: None,
            });
            x += small_cell * 2.0;
        }

        if tab.rang {
            frame.lines.push(TextLine {
                x,
                y,
                small: true,
                spans: vec![Span::plain("!", theme.accent)],
                clip: None,
            });
            x += small_cell * 2.0;
        }

        let room = rect.x + rect.width - CLOSE_ZONE * scale - x;
        let title = if tab.confirming {
            "Press again to end it"
        } else {
            tab.title.as_str()
        };
        let color = match (tab.confirming, is_active) {
            (true, _) => theme.failure,
            (false, true) => theme.foreground,
            (false, false) => theme.muted,
        };
        frame.lines.push(TextLine {
            x,
            y,
            small: true,
            spans: vec![Span::plain(fit(title, (room / small_cell) as usize), color)],
            clip: None,
        });

        if hovered || is_active {
            let color = if hovered && chrome.hovered_close {
                theme.failure
            } else {
                theme.muted
            };
            frame.lines.push(TextLine {
                x: rect.x + rect.width - 20.0 * scale,
                y,
                small: true,
                spans: vec![Span::plain("×", color)],
                clip: None,
            });
        }
    }

    let gear = chrome.bar.settings;
    let lit = chrome.hovered_settings || chrome.settings_open;
    if lit {
        let inset = 6.0 * scale;
        frame.quads.push(Quad {
            x: gear.x + inset,
            y: top + inset / 2.0,
            width: gear.width - inset * 2.0,
            height: strip - top - inset,
            color: rgba(theme.foreground, 0.1),
            shape: Shape::Rounded {
                radius: 6.0 * scale,
            },
        });
    }
    let radius = 9.0 * scale;
    frame.quads.push(Quad {
        x: gear.x + gear.width / 2.0 - radius,
        y: (top + strip) / 2.0 - radius,
        width: radius * 2.0,
        height: radius * 2.0,
        color: rgba(if lit { theme.foreground } else { theme.muted }, 1.0),
        shape: Shape::Gear { radius },
    });

    let plus = chrome.bar.new_tab;
    let color = if chrome.hovered_new {
        theme.foreground
    } else {
        theme.muted
    };
    frame.lines.push(TextLine {
        x: plus.x + (plus.width - layout.cell.width) / 2.0,
        y: (top + strip - layout.cell.height) / 2.0,
        small: false,
        spans: vec![Span::plain("+", color)],
        clip: None,
    });
}

/// The strip is a shade laid over the window, with the active tab cut out of it,
/// so the tab flows into the terminal with no seam at any opacity.
fn shade_around_tab(frame: &mut Frame, tab: Rect, width: f32, strip: f32, top: f32, radius: f32) {
    let shade = STRIP_SHADE;
    let (left, right) = (tab.x, tab.x + tab.width);
    let tall = strip - top;
    let piece = |x, y, w, h, shape| Quad {
        x,
        y,
        width: w,
        height: h,
        color: shade,
        shape,
    };

    frame.quads.extend([
        Quad::new(0.0, 0.0, width, top, shade),
        Quad::new(0.0, top, left - radius, tall, shade),
        Quad::new(right + radius, top, width - right - radius, tall, shade),
        Quad::new(left - radius, top, radius, tall - radius, shade),
        Quad::new(right, top, radius, tall - radius, shade),
        piece(
            left - radius,
            strip - radius,
            radius,
            radius,
            Shape::InsideCircle {
                x: left - radius,
                y: strip - radius,
                radius,
            },
        ),
        piece(
            right,
            strip - radius,
            radius,
            radius,
            Shape::InsideCircle {
                x: right + radius,
                y: strip - radius,
                radius,
            },
        ),
        piece(
            left,
            top,
            radius,
            radius,
            Shape::OutsideCircle {
                x: left + radius,
                y: top + radius,
                radius,
            },
        ),
        piece(
            right - radius,
            top,
            radius,
            radius,
            Shape::OutsideCircle {
                x: right - radius,
                y: top + radius,
                radius,
            },
        ),
    ]);
}

fn fit(text: &str, room: usize) -> String {
    if text.chars().count() <= room {
        return text.to_string();
    }
    let kept: String = text.chars().take(room.saturating_sub(1)).collect();
    format!("{kept}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consent_banner_shows_prompt_and_gesture() {
        let layout = Layout {
            width: 600.0,
            height: 400.0,
            scale: 1.0,
            padding: 8.0,
            cell: Cell {
                width: 8.0,
                height: 20.0,
            },
        };
        let mut frame = Frame::default();
        draw_consent(
            &mut frame,
            "Allow clipboard write?",
            layout,
            &Theme::default(),
        );

        assert!(frame.lines.iter().any(|line| line.spans.iter().any(|span| {
            span.text.contains("Allow clipboard write?")
                && span.text.contains("Ctrl+Shift+Enter")
                && span.text.contains("Esc")
        })));
        assert!(frame.quads.iter().any(|quad| quad.width == layout.width
            && quad.y == layout.strip()
            && quad.height > layout.cell.height));
    }
}

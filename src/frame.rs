use std::time::Duration;

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Point};
use alacritty_terminal::term::search::Match;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};

use crate::boxes;
use crate::blocks::{Activity, Block, BlockStore};
use crate::format;
use crate::config::Config;
use crate::graphics::Graphics;
use crate::links::Link;
use crate::render::{Cell, Clip, Picture, Pictures, Quad, Shape, Span, TextLine, rgba};
use crate::settings::{FIELDS, Kind, Panel, Settings, value};
use crate::tabs::{Bar, CLOSE_ZONE, Rect};
use crate::theme::Theme;
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

    pub fn columns(&self) -> usize {
        (((self.width - self.grid_x() - self.left()) / self.cell.width) as usize).max(2)
    }

    pub fn lines(&self) -> usize {
        (((self.height - self.grid_y() - self.left()) / self.cell.height) as usize).max(1)
    }

    /// The visible (row, column) under a window position.
    pub fn cell_at(&self, x: f32, y: f32) -> (usize, usize) {
        let row = ((y - self.grid_y()).max(0.0) / self.cell.height) as usize;
        let column = ((x - self.grid_x()).max(0.0) / self.cell.width) as usize;
        (row.min(self.lines() - 1), column.min(self.columns() - 1))
    }

    pub fn in_gutter(&self, x: f32) -> bool {
        x < self.grid_x()
    }
}

#[derive(Default)]
pub struct Frame {
    pub quads: Vec<Quad>,
    pub lines: Vec<TextLine>,
    pub pictures: Pictures,
    pub clip: Clip,
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

/// The marks of one cell. Dots and curls follow the window position, so neighbours join.
pub fn underline(style: Underline, x: f32, y: f32, width: f32, thickness: f32, color: [f32; 4]) -> Vec<Quad> {
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
            vec![Quad::new(x, y, width, thickness, color), Quad::new(x, y + 2.0 * thickness, width, thickness, color)]
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
                quads.push(Quad::new(left, y + wave(left.floor()).round(), right - left, thickness, color));
                left = right;
            }
            quads
        }
    }
}

pub fn visible_top(rows: &[Row], lines: usize, scroll: usize) -> usize {
    rows.len().saturating_sub(lines + scroll)
}

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
    let top = visible_top(rows, layout.lines(), scroll);
    let selection = term.selection.as_ref().and_then(|selection| selection.to_range(term));
    let grid = term.grid();
    let columns = grid.columns();
    let colors = term.colors();
    let default_bg = theme.resolve(Color::Named(NamedColor::Background), colors);

    for (index, row) in rows[top..].iter().take(layout.lines()).enumerate() {
        let y = layout.grid_y() + index as f32 * layout.cell.height;
        let cells = &grid[row.line];
        let mut spans: Vec<Span> = Vec::new();

        for column in 0..columns {
            let cell = &cells[Column(column)];
            if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }

            let mut fg = theme.resolve(cell.fg, colors);
            let mut bg = theme.resolve(cell.bg, colors);
            if cell.flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            if cell.flags.contains(Flags::DIM) {
                fg = Rgb { r: fg.r / 3 * 2, g: fg.g / 3 * 2, b: fg.b / 3 * 2 };
            }
            if cell.flags.contains(Flags::HIDDEN) {
                fg = bg;
            }

            let x = layout.grid_x() + column as f32 * layout.cell.width;
            let wide = if cell.flags.contains(Flags::WIDE_CHAR) { 2.0 } else { 1.0 };
            let width = layout.cell.width * wide;
            let selected = selection.is_some_and(|range| range.contains(Point::new(row.line, Column(column))));
            if selected {
                frame.quads.push(Quad::new(x, y, width, layout.cell.height, rgba(theme.accent, 0.35)));
            } else if bg != default_bg {
                frame.quads.push(Quad::new(x, y, width, layout.cell.height, rgba(bg, 1.0)));
            }
            let thickness = layout.scale.max(1.0);
            if let Some(style) = underline_style(cell.flags) {
                let color = cell.underline_color().map_or(fg, |color| theme.resolve(color, colors));
                let top = y + layout.cell.height - 3.0 * thickness;
                frame.quads.extend(underline(style, x, top, width, thickness, rgba(color, 1.0)));
            }
            if cell.flags.contains(Flags::STRIKEOUT) {
                let strike = y + layout.cell.height / 2.0;
                frame.quads.push(Quad::new(x, strike, width, thickness, rgba(fg, 1.0)));
            }

            let mut text = String::from(if cell.c == '\0' { ' ' } else { cell.c });
            if let Some(extra) = cell.zerowidth() {
                text.extend(extra.iter());
            }
            let mut text = without_anchors(&text);
            if let Some(quads) = boxes::quads(cell.c, x, y, width, layout.cell.height, thickness, rgba(fg, 1.0)) {
                frame.quads.extend(quads);
                text = " ".to_string();
            }
            let bold = cell.flags.intersects(Flags::BOLD);
            let italic = cell.flags.intersects(Flags::ITALIC);
            let color = (fg.r, fg.g, fg.b);
            match spans.last_mut() {
                Some(last) if last.color == color && last.bold == bold && last.italic == italic => {
                    last.text.push_str(&text);
                }
                _ => spans.push(Span { text, color, bold, italic }),
            }
        }

        while spans.last().is_some_and(|span| span.text.trim_end().is_empty()) {
            spans.pop();
        }
        if let Some(last) = spans.last_mut() {
            last.text.truncate(last.text.trim_end().len());
            frame.lines.push(TextLine { x: layout.grid_x(), y, small: false, spans });
        }

        if let Some(block) = row.block.and_then(|id| blocks.get(id)) {
            draw_block_marks(frame, block, row, y, blocks, layout, theme);
        }
    }

    frame.pictures.over = frame.quads.len();

    if scroll == 0 && cursor.visible && term.mode().contains(TermMode::SHOW_CURSOR) {
        let point = grid.cursor.point;
        let position = rows[top..].iter().take(layout.lines()).position(|row| row.line == point.line);
        if let Some(index) = position {
            draw_cursor(frame, term, index, point.column.0, cursor.focused, layout, theme);
        }
    }

    if let Some(block) = pinned(rows, top, blocks).and_then(|id| blocks.get(id)) {
        draw_pinned(frame, block, layout, theme);
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
    let cell = layout.cell;
    for found in matches {
        let (start, end) = (found.start(), found.end());
        for (index, row) in rows.iter().enumerate().skip(top).take(layout.lines()) {
            if row.line < start.line || row.line > end.line {
                continue;
            }
            let first = if row.line == start.line { start.column.0 } else { 0 };
            let last = if row.line == end.line { end.column.0 + 1 } else { layout.columns() };
            let (x, y) = (layout.grid_x() + first as f32 * cell.width, layout.grid_y() + (index - top) as f32 * cell.height);
            let width = last.saturating_sub(first) as f32 * cell.width;
            let focused = current == Some(found);
            let alpha = if focused { 0.6 } else { 0.25 };
            frame.quads.push(Quad::new(x, y, width, cell.height, rgba(theme.accent, alpha)));
            if focused {
                let line = layout.scale.max(1.0);
                let ink = rgba(theme.foreground, 1.0);
                frame.quads.push(Quad::new(x, y, width, line, ink));
                frame.quads.push(Quad::new(x, y + cell.height - line, width, line, ink));
                frame.quads.push(Quad::new(x, y, line, cell.height, ink));
                frame.quads.push(Quad::new(x + width - line, y, line, cell.height, ink));
            }
        }
    }
}

/// A panel in the bottom right corner. Gives its x, width and the y of its text.
fn bottom_panel(frame: &mut Frame, columns: f32, layout: Layout, theme: &Theme) -> (f32, f32, f32) {
    let scale = layout.scale;
    let cell = layout.cell;
    let width = (columns * cell.width + PANEL_INSET * scale * 2.0).min(layout.width - layout.left() * 2.0);
    let height = cell.height + 12.0 * scale;
    let (x, y) = (layout.width - layout.left() - width, layout.height - layout.left() - height);
    frame.clip = Clip { count: frame.lines.len(), x, top: y - cell.height, bottom: y + height };
    frame.quads.push(Quad {
        x,
        y,
        width,
        height,
        color: rgba(theme.ansi[0], 1.0),
        shape: Shape::Rounded { radius: 10.0 * scale },
    });
    (x, width, y + (height - cell.height) / 2.0)
}

pub fn draw_search_bar(frame: &mut Frame, query: &str, found: bool, layout: Layout, theme: &Theme) {
    let (scale, cell) = (layout.scale, layout.cell);
    let inset = PANEL_INSET * scale;
    let (x, width, text_y) = bottom_panel(frame, SEARCH_COLUMNS, layout, theme);
    let status = if found || query.is_empty() { "" } else { "No matches" };
    let room = (((width - inset * 2.0) / cell.width) as usize).saturating_sub(status.chars().count());
    let shown = fit(query, room.saturating_sub(1));
    let caret = x + inset + shown.chars().count() as f32 * cell.width;
    frame.quads.push(Quad::new(caret, text_y, (2.0 * scale).round(), cell.height, rgba(theme.cursor, 0.9)));
    let spans = if query.is_empty() {
        vec![Span::plain("Search", theme.muted)]
    } else {
        vec![Span::plain(shown, theme.foreground)]
    };
    frame.lines.push(TextLine { x: x + inset, y: text_y, small: false, spans });
    if !status.is_empty() {
        let status_x = x + width - inset - status.chars().count() as f32 * cell.width;
        frame.lines.push(TextLine { x: status_x, y: text_y, small: false, spans: vec![Span::plain(status, theme.failure)] });
    }
}

pub fn draw_link(frame: &mut Frame, rows: &[Row], top: usize, link: &Link, layout: Layout, theme: &Theme) {
    let (start, end) = (link.range.start(), link.range.end());
    let cell = layout.cell;
    let line = layout.scale.max(1.0);
    for (index, row) in rows.iter().enumerate().skip(top).take(layout.lines()) {
        if row.line < start.line || row.line > end.line {
            continue;
        }
        let first = if row.line == start.line { start.column.0 } else { 0 };
        let last = if row.line == end.line { end.column.0 + 1 } else { layout.columns() };
        let x = layout.grid_x() + first as f32 * cell.width;
        let y = layout.grid_y() + (index - top + 1) as f32 * cell.height - 2.0 * line;
        frame.quads.push(Quad::new(x, y, last.saturating_sub(first) as f32 * cell.width, line, rgba(theme.accent, 1.0)));
    }
}

/// The real target, since the text on screen can say anything.
pub fn draw_link_bar(frame: &mut Frame, uri: &str, layout: Layout, theme: &Theme) {
    let cell = layout.cell;
    let inset = PANEL_INSET * layout.scale;
    let (x, width, y) = bottom_panel(frame, uri.chars().count() as f32, layout, theme);
    let shown = fit(uri, ((width - inset * 2.0) / cell.width) as usize);
    frame.lines.push(TextLine { x: x + inset, y, small: false, spans: vec![Span::plain(shown, theme.foreground)] });
}

/// Keeps the image's shape inside the cells it was given.
// shortcut: c and r that ask for a stretch are ignored, honour them if a program needs it.
pub fn fit_picture(image: (u32, u32), area: (f32, f32)) -> (f32, f32) {
    let (width, height) = (image.0 as f32, image.1 as f32);
    let scale = (area.0 / width).min(area.1 / height);
    (width * scale, height * scale)
}

pub fn draw_pictures<L: EventListener>(
    frame: &mut Frame,
    term: &Term<L>,
    graphics: &Graphics,
    rows: &[Row],
    scroll: usize,
    layout: Layout,
) {
    let cell = layout.cell;
    let top = visible_top(rows, layout.lines(), scroll);
    for found in placed(term, graphics, rows, top, layout.lines()) {
        let Some(image) = graphics.image(found.image) else { continue };
        let area = (found.columns as f32 * cell.width, found.rows as f32 * cell.height);
        let (width, height) = fit_picture((image.width, image.height), area);
        let x = layout.grid_x() + found.column as f32 * cell.width;
        let y = layout.grid_y() + found.row as f32 * cell.height;
        frame.pictures.list.push(Picture { image: image.clone(), x, y, width, height });
    }
    let area = (layout.columns() as f32 * cell.width, layout.lines() as f32 * cell.height);
    frame.pictures.area = [layout.grid_x(), layout.grid_y(), area.0, area.1];
}

fn draw_block_marks(
    frame: &mut Frame,
    block: &Block,
    row: &Row,
    y: f32,
    blocks: &BlockStore,
    layout: Layout,
    theme: &Theme,
) {
    if block.status.is_none() && block.command.is_none() {
        return;
    }
    let toggle = if block.folded { "▸" } else { "▾" };
    let toggle_color = match block.status {
        Some(0) => theme.success,
        Some(_) => theme.failure,
        None => theme.accent,
    };
    frame.lines.push(TextLine {
        x: layout.left(),
        y,
        small: false,
        spans: vec![Span::plain(toggle, toggle_color)],
    });

    let running = blocks.activity() == Activity::Running && blocks.latest().is_some_and(|last| last.id == block.id);
    let mut spans = Vec::new();
    if row.hidden > 0 {
        spans.push(Span::plain(format!("{}  ", format::lines(row.hidden)), theme.muted));
    }
    match block.status {
        Some(0) => spans.push(Span::plain("✓", theme.success)),
        Some(status) => spans.push(Span::plain(format!("✗ {status}"), theme.failure)),
        None if running => spans.push(Span::plain("running", theme.accent)),
        None => {}
    }
    if let Some(duration) = block.duration.filter(|value| *value >= Duration::from_millis(100)) {
        spans.push(Span::plain(format!("  {}", format::duration(duration)), theme.muted));
    }
    if spans.is_empty() {
        return;
    }

    let characters: usize = spans.iter().map(|span| span.text.chars().count()).sum();
    let x = layout.width - layout.left() - characters as f32 * layout.cell.width * 0.82;
    frame.lines.push(TextLine { x, y, small: true, spans });
}

/// Where the cursor sits on screen, when it is on screen.
pub fn cursor_position<L: EventListener>(term: &Term<L>, rows: &[Row], scroll: usize, layout: Layout) -> Option<(f32, f32)> {
    let top = visible_top(rows, layout.lines(), scroll);
    let cursor = term.grid().cursor.point;
    let index = rows[top..].iter().take(layout.lines()).position(|row| row.line == cursor.line)?;
    let x = layout.grid_x() + cursor.column.0 as f32 * layout.cell.width;
    Some((x, layout.grid_y() + index as f32 * layout.cell.height))
}

/// Text an input method is still composing.
// shortcut: counts characters as one cell each, wide scripts underline short.
pub fn draw_preedit(frame: &mut Frame, text: &str, x: f32, y: f32, layout: Layout, theme: &Theme) {
    let width = text.chars().count() as f32 * layout.cell.width;
    let thickness = layout.scale.max(1.0);
    frame.quads.push(Quad::new(x, y + layout.cell.height - 2.0 * thickness, width, thickness, rgba(theme.accent, 1.0)));
    frame.lines.push(TextLine { x, y, small: false, spans: vec![Span::plain(text.to_string(), theme.foreground)] });
}

fn draw_cursor<L>(
    frame: &mut Frame,
    term: &Term<L>,
    index: usize,
    column: usize,
    focused: bool,
    layout: Layout,
    theme: &Theme,
) {
    let x = layout.grid_x() + column as f32 * layout.cell.width;
    let y = layout.grid_y() + index as f32 * layout.cell.height;
    let thickness = (2.0 * layout.scale).round();
    let color = rgba(theme.cursor, if focused { 0.9 } else { 0.4 });
    let (width, height) = (layout.cell.width, layout.cell.height);
    frame.quads.push(match term.cursor_style().shape {
        CursorShape::Beam => Quad::new(x, y, thickness, height, color),
        CursorShape::Underline => Quad::new(x, y + height - thickness, width, thickness, color),
        CursorShape::Hidden => return,
        _ => Quad::new(x, y, width, height, color),
    });
}

fn draw_pinned(frame: &mut Frame, block: &Block, layout: Layout, theme: &Theme) {
    let y = layout.grid_y();
    frame.quads.push(Quad::new(0.0, y, layout.width, layout.cell.height, rgba(theme.ansi[0], 1.0)));
    let command = block.command.as_deref().and_then(|text| text.lines().next()).unwrap_or("running");
    frame.lines.push(TextLine {
        x: layout.left(),
        y,
        small: false,
        spans: vec![Span::plain(format!("▾ {command}"), theme.foreground)],
    });
}

pub fn panel(layout: Layout) -> Panel {
    let scale = layout.scale;
    let width = (420.0 * scale).min(layout.width - 16.0 * scale);
    Panel {
        x: layout.width - width - 8.0 * scale,
        y: layout.strip() + 6.0 * scale,
        width,
        row_height: (layout.cell.height + 12.0 * scale).round(),
    }
}

pub fn draw_settings(
    frame: &mut Frame,
    settings: &Settings,
    config: &Config,
    error: Option<&str>,
    layout: Layout,
    theme: &Theme,
) {
    let area = panel(layout);
    let scale = layout.scale;
    let inset = 14.0 * scale;
    let bottom = area.y + area.height();
    frame.clip = Clip { count: frame.lines.len(), x: area.x, top: area.y - layout.cell.height, bottom };
    frame.quads.push(Quad {
        x: area.x,
        y: area.y,
        width: area.width,
        height: area.height(),
        color: rgba(theme.ansi[0], 1.0),
        shape: Shape::Rounded { radius: 10.0 * scale },
    });

    let small_cell = layout.cell.width * 0.82;
    for (index, field) in FIELDS.iter().enumerate() {
        let top = area.y + index as f32 * area.row_height;
        let y = top + (area.row_height - layout.cell.height) / 2.0;
        let selected = index == settings.selected;
        if selected {
            frame.quads.push(Quad {
                x: area.x + 4.0 * scale,
                y: top + 2.0 * scale,
                width: area.width - 8.0 * scale,
                height: area.row_height - 4.0 * scale,
                color: rgba(theme.foreground, 0.08),
                shape: Shape::Rounded { radius: 7.0 * scale },
            });
        }
        let label = if selected { theme.foreground } else { theme.muted };
        frame.lines.push(TextLine {
            x: area.x + inset,
            y,
            small: true,
            spans: vec![Span::plain(field.label, label)],
        });

        let right = area.x + area.width - inset;
        match (field.kind, settings.editing.as_ref().filter(|_| selected)) {
            (Kind::Number { .. }, _) => {
                let text = value(index, config);
                let arrows = [(area.less_x(), "‹"), (area.x + area.width - area.arrow_width(), "›")];
                for (x, arrow) in arrows {
                    frame.lines.push(TextLine {
                        x: x + (area.arrow_width() - small_cell) / 2.0,
                        y,
                        small: true,
                        spans: vec![Span::plain(arrow, theme.accent)],
                    });
                }
                let width = text.chars().count() as f32 * small_cell;
                let middle = area.x + area.width - 2.5 * area.arrow_width();
                frame.lines.push(TextLine {
                    x: middle - width / 2.0,
                    y,
                    small: true,
                    spans: vec![Span::plain(text, theme.foreground)],
                });
            }
            (Kind::Toggle, _) => {
                let on = value(index, config) == "on";
                let (width, height, gap) = (30.0 * scale, 16.0 * scale, 3.0 * scale);
                let track = top + (area.row_height - height) / 2.0;
                let knob = height - gap * 2.0;
                frame.quads.push(Quad {
                    x: right - width,
                    y: track,
                    width,
                    height,
                    color: if on { rgba(theme.accent, 1.0) } else { rgba(theme.muted, 0.35) },
                    shape: Shape::Rounded { radius: height / 2.0 },
                });
                frame.quads.push(Quad {
                    x: if on { right - gap - knob } else { right - width + gap },
                    y: track + gap,
                    width: knob,
                    height: knob,
                    color: rgba(if on { theme.background } else { theme.foreground }, 1.0),
                    shape: Shape::Rounded { radius: knob / 2.0 },
                });
            }
            (Kind::Text, editing) => {
                let room = ((area.width * 0.55 - inset) / small_cell) as usize;
                let text = editing.cloned().unwrap_or_else(|| value(index, config));
                let shown = if text.is_empty() && editing.is_none() {
                    "Default".to_string()
                } else {
                    fit(&text, room)
                };
                let width = shown.chars().count() as f32 * small_cell;
                let color = if editing.is_some() { theme.foreground } else { theme.muted };
                frame.lines.push(TextLine {
                    x: right - width,
                    y,
                    small: true,
                    spans: vec![Span::plain(shown, color)],
                });
                if editing.is_some() {
                    let caret = Quad::new(right + scale, y + 3.0 * scale, 2.0 * scale, layout.cell.height - 6.0 * scale, rgba(theme.accent, 1.0));
                    frame.quads.push(caret);
                }
            }
        }
    }

    let toggle = FIELDS[settings.selected].kind == Kind::Toggle;
    let (hint, color) = match (error, settings.editing.is_some()) {
        (Some(message), _) => (message, theme.failure),
        (None, true) => ("Enter saves. Esc cancels.", theme.muted),
        (None, false) if toggle => ("Colours update on each matugen run.", theme.muted),
        (None, false) => ("Arrows change a setting. Esc closes.", theme.muted),
    };
    let room = ((area.width - inset * 2.0) / small_cell) as usize;
    frame.lines.push(TextLine {
        x: area.x + inset,
        y: bottom - area.row_height + (area.row_height - layout.cell.height) / 2.0,
        small: true,
        spans: vec![Span::plain(fit(hint, room), color)],
    });
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
                shape: Shape::Rounded { radius: 6.0 * scale },
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
            });
            x += small_cell * 2.0;
        }

        if tab.rang {
            frame.lines.push(TextLine { x, y, small: true, spans: vec![Span::plain("!", theme.accent)] });
            x += small_cell * 2.0;
        }

        let room = rect.x + rect.width - CLOSE_ZONE * scale - x;
        let title = if tab.confirming { "Press again to end it" } else { tab.title.as_str() };
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
        });

        if hovered || is_active {
            let color = if hovered && chrome.hovered_close { theme.failure } else { theme.muted };
            frame.lines.push(TextLine {
                x: rect.x + rect.width - 20.0 * scale,
                y,
                small: true,
                spans: vec![Span::plain("×", color)],
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
            shape: Shape::Rounded { radius: 6.0 * scale },
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
    let color = if chrome.hovered_new { theme.foreground } else { theme.muted };
    frame.lines.push(TextLine {
        x: plus.x + (plus.width - layout.cell.width) / 2.0,
        y: (top + strip - layout.cell.height) / 2.0,
        small: false,
        spans: vec![Span::plain("+", color)],
    });
}

/// The strip is a shade laid over the window, with the active tab cut out of it,
/// so the tab flows into the terminal with no seam at any opacity.
fn shade_around_tab(frame: &mut Frame, tab: Rect, width: f32, strip: f32, top: f32, radius: f32) {
    let shade = STRIP_SHADE;
    let (left, right) = (tab.x, tab.x + tab.width);
    let tall = strip - top;
    let piece = |x, y, w, h, shape| Quad { x, y, width: w, height: h, color: shade, shape };

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
            Shape::InsideCircle { x: left - radius, y: strip - radius, radius },
        ),
        piece(
            right,
            strip - radius,
            radius,
            radius,
            Shape::InsideCircle { x: right + radius, y: strip - radius, radius },
        ),
        piece(
            left,
            top,
            radius,
            radius,
            Shape::OutsideCircle { x: left + radius, y: top + radius, radius },
        ),
        piece(
            right - radius,
            top,
            radius,
            radius,
            Shape::OutsideCircle { x: right - radius, y: top + radius, radius },
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

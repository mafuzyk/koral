use std::time::Instant;

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::index::{Column, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use koral::frame::visible_top;
use koral::session::Session;
use koral::view;

/// Drags the way `point_under_cursor` and `cursor_moved` in main.rs do: the window
/// row goes through `view::rows` and `frame::visible_top` before becoming a `Point`.
struct Fixture {
    session: Session<VoidListener>,
    lines: usize,
    now: Instant,
}

impl Fixture {
    fn new(columns: usize, lines: usize) -> Self {
        let config = Config {
            scrolling_history: 100,
            ..Config::default()
        };
        let term = Term::new(config, &TermSize::new(columns, lines), VoidListener);
        Self {
            session: Session::new(term),
            lines,
            now: Instant::now(),
        }
    }

    fn feed(&mut self, text: &str) {
        self.session.feed(text.as_bytes(), self.now);
    }

    fn point(&self, row: usize, column: usize) -> Point {
        let rows = view::rows(&self.session.term, &self.session.blocks);
        let top = visible_top(&rows, self.lines, 0);
        let line = rows[top + row].line;
        Point::new(line, Column(column))
    }

    fn drag(&mut self, from: (usize, usize), to: (usize, usize)) -> Option<String> {
        let mut selection = Selection::new(
            SelectionType::Simple,
            self.point(from.0, from.1),
            Side::Left,
        );
        selection.update(self.point(to.0, to.1), Side::Right);
        self.session.term.selection = Some(selection);
        self.session.term.selection_to_string()
    }
}

#[test]
fn a_drag_selects_on_the_primary_screen() {
    let mut fixture = Fixture::new(20, 6);
    fixture.feed("hello\r\nworld\r\n");
    assert_eq!(
        fixture.drag((0, 0), (1, 4)).as_deref(),
        Some("hello\nworld")
    );
}

#[test]
fn a_drag_selects_on_the_alternate_screen() {
    let mut fixture = Fixture::new(20, 6);
    fixture.feed("\x1b[?1049h");
    fixture.feed("hello\r\nworld\r\n");
    assert_eq!(
        fixture.drag((0, 0), (1, 4)).as_deref(),
        Some("hello\nworld")
    );
}

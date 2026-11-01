use std::time::Instant;

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::index::{Column, Direction, Line, Point};
use alacritty_terminal::term::Term;
use alacritty_terminal::term::test::TermSize;
use koral::pty::term_config;
use koral::search::{Search, escape};
use koral::session::Session;
use koral::view;

fn session(text: &str) -> Session<VoidListener> {
    let term = Term::new(term_config(), &TermSize::new(20, 5), VoidListener);
    let mut session = Session::new(term);
    session.feed(text.as_bytes(), Instant::now());
    session
}

fn search(query: &str) -> Search {
    let mut search = Search::default();
    search.set_query(query.to_string());
    search
}

fn first(session: &Session<VoidListener>, query: &str) -> Option<Point> {
    search(query).step(&session.term, Direction::Left).map(|found| *found.start())
}

fn line(number: i32, column: usize) -> Point {
    Point::new(Line(number), Column(column))
}

#[test]
fn escape_makes_a_query_plain_text() {
    assert_eq!(escape("a.b(c)"), r"a\.b\(c\)");
}

#[test]
fn a_dot_matches_only_a_dot() {
    assert_eq!(first(&session("ab\r\nc.d\r\n"), "."), Some(line(1, 1)));
}

#[test]
fn finds_a_word_in_the_scrollback() {
    let text: String = (0..200).map(|index| format!("{}\r\n", if index == 3 { "needle" } else { "hay" })).collect();
    assert_eq!(first(&session(&text), "needle"), Some(line(-193, 0)));
}

#[test]
fn steps_go_up_and_wrap() {
    let session = session("one\r\ntwo one\r\n");
    let mut search = search("one");
    let mut step = || search.step(&session.term, Direction::Left).map(|found| *found.start());
    assert_eq!(step(), Some(line(1, 4)));
    assert_eq!(step(), Some(line(0, 0)));
    assert_eq!(step(), Some(line(1, 4)));
}

#[test]
fn steps_go_down_again() {
    let session = session("one\r\ntwo one\r\n");
    let mut search = search("one");
    search.step(&session.term, Direction::Left);
    search.step(&session.term, Direction::Left);
    let found = search.step(&session.term, Direction::Right);
    assert_eq!(found.map(|found| *found.start()), Some(line(1, 4)));
}

#[test]
fn lowercase_ignores_case() {
    assert_eq!(first(&session("NEEDLE\r\n"), "needle"), Some(line(0, 0)));
}

#[test]
fn uppercase_is_exact() {
    assert_eq!(first(&session("needle\r\n"), "Needle"), None);
}

#[test]
fn anchors_do_not_break_a_match() {
    let text = "\x1b]133;A\x07$ \x1b]133;B\x07needle\x1b]133;C\x07\r\n";
    assert_eq!(first(&session(text), "needle"), Some(line(0, 2)));
}

#[test]
fn a_match_in_a_fold_names_its_block() {
    let text = "\x1b]133;A\x07$ \x1b]133;B\x07ls\x1b]133;C\x07\r\nneedle\r\n\x1b]133;D;0\x07";
    let mut session = session(text);
    let id = session.blocks.latest().unwrap().id;
    session.blocks.toggle_fold(id);
    let rows = view::rows(&session.term, &session.blocks);
    assert_eq!(view::row_of(&rows, Line(1)), None);
    assert_eq!(view::block_of(&rows, Line(1)), Some(id));
    assert_eq!(view::block_of(&rows, Line(0)), None);
}

#[test]
fn no_match_is_none() {
    assert_eq!(first(&session("hay\r\n"), "needle"), None);
}

#[test]
fn an_empty_query_finds_nothing() {
    assert_eq!(first(&session("hay\r\n"), ""), None);
}

#[test]
fn only_what_is_in_view_is_marked() {
    let session = session("one\r\ntwo\r\none\r\n");
    let found = search("one").visible(&session.term, Line(1), Line(2));
    assert_eq!(found.iter().map(|hit| *hit.start()).collect::<Vec<_>>(), vec![line(2, 0)]);
}

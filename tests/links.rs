use std::time::Instant;

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::Term;
use alacritty_terminal::term::test::TermSize;
use koral::links::{Link, allowed, at};
use koral::pty::term_config;
use koral::session::Session;

fn point(line: i32, column: usize) -> Point {
    Point::new(Line(line), Column(column))
}

fn link(text: &str, under: Point) -> Option<Link> {
    let term = Term::new(term_config(), &TermSize::new(20, 5), VoidListener);
    let mut session = Session::new(term);
    session.feed(text.as_bytes(), Instant::now());
    at(&session.term, under)
}

#[test]
fn finds_a_plain_url() {
    let found = link("go https://a.b/c now", point(0, 6)).unwrap();
    assert_eq!(found.uri, "https://a.b/c");
    assert_eq!(found.range, point(0, 3)..=point(0, 15));
}

#[test]
fn drops_trailing_punctuation() {
    assert_eq!(link("see https://a.b/c.", point(0, 8)).unwrap().uri, "https://a.b/c");
}

#[test]
fn trailing_punctuation_is_not_a_link() {
    assert!(link("see https://a.b/c.", point(0, 17)).is_none());
}

#[test]
fn keeps_a_url_across_a_wrapped_line() {
    let url = format!("https://a.example/{}", "x".repeat(10));
    let found = link(&url, point(1, 3)).unwrap();
    assert_eq!(found.uri, url);
    assert_eq!(*found.range.start(), point(0, 0));
    assert_eq!(*found.range.end(), point(1, 7));
}

#[test]
fn finds_an_osc8_link() {
    let found = link("a \x1b]8;;https://real.example\x1b\\click\x1b]8;;\x1b\\ b", point(0, 4)).unwrap();
    assert_eq!(found.uri, "https://real.example");
    assert_eq!(found.range, point(0, 2)..=point(0, 6));
}

#[test]
fn osc8_beats_the_text_it_shows() {
    let text = "\x1b]8;;https://real.example\x1b\\https://fake.example\x1b]8;;\x1b\\";
    assert_eq!(link(text, point(0, 3)).unwrap().uri, "https://real.example");
}

#[test]
fn no_link_on_plain_text() {
    assert!(link("plain words here", point(0, 3)).is_none());
}

#[test]
fn only_safe_schemes_are_allowed() {
    for uri in ["https://a", "http://a", "mailto:a@b", "file:///a", "HTTPS://a"] {
        assert!(allowed(uri), "{uri}");
    }
    for uri in ["javascript:alert(1)", "data:text/html,x", "ssh://a", "-x", ""] {
        assert!(!allowed(uri), "{uri}");
    }
}

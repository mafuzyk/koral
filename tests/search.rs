use alacritty_terminal::event::VoidListener;
use alacritty_terminal::index::{Direction, Line};
use alacritty_terminal::term::Term;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::vte::ansi::Processor;
use koral::search::Search;

fn term_with_lines(lines: &[&str]) -> Term<VoidListener> {
    let size = TermSize::new(40, lines.len().max(4));
    let mut term = Term::new(koral::pty::term_config(), &size, VoidListener);
    let mut parser: Processor = Processor::new();
    for (idx, line) in lines.iter().enumerate() {
        parser.advance(&mut term, line.as_bytes());
        if idx + 1 < lines.len() {
            parser.advance(&mut term, b"\r\n");
        }
    }
    term
}

#[test]
fn scoped_search_only_matches_within_block_bounds() {
    let term = term_with_lines(&[
        "match outside top",
        "match inside one",
        "no hit",
        "match inside two",
        "match outside bottom",
    ]);
    let mut search = Search::default();
    search.set_scope(Some((Line(1), Line(3))));
    search.set_query("match".to_string());

    let first = search.step(&term, Direction::Right).expect("first match");
    assert_eq!(first.start().line, Line(1));
    assert_eq!(search.position, Some((1, 2)));

    let second = search.step(&term, Direction::Right).expect("second match");
    assert_eq!(second.start().line, Line(3));
    assert_eq!(search.position, Some((2, 2)));

    let wrapped = search.step(&term, Direction::Right).expect("wrapped");
    assert_eq!(wrapped.start().line, Line(1));

    assert!(search.visible(&term, Line(0), Line(0)).is_empty());
    assert_eq!(search.visible(&term, Line(0), Line(4)).len(), 2);
}

use std::process::Command;

use alacritty_terminal::event::EventListener;
use alacritty_terminal::index::{Boundary, Direction, Point};
use alacritty_terminal::term::Term;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::search::{Match, RegexIter, RegexSearch};

const SCHEMES: [&str; 4] = ["http", "https", "mailto", "file"];
const URL: &str = r#"(https?://|mailto:|file:)[^\x00-\x1F\x7F-\x9F<>"\s{}\^`]+"#;
// shortcut: a closing bracket at the end is always dropped, keep it when the URL opens one if a site needs it.
const TRAILING: &str = ".,;:!?)";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub uri: String,
    pub range: Match,
}

pub fn at<L: EventListener>(term: &Term<L>, point: Point) -> Option<Link> {
    hyperlink(term, point).or_else(|| plain(term, point))
}

/// The cells around `point` that carry the same OSC 8 link.
fn hyperlink<L: EventListener>(term: &Term<L>, point: Point) -> Option<Link> {
    let grid = term.grid();
    let link = grid[point].hyperlink()?;
    let same = |at: Point| grid[at].hyperlink().as_ref() == Some(&link);
    let walk = |step: &dyn Fn(Point) -> Point| {
        let mut edge = point;
        loop {
            let next = step(edge);
            if next == edge || !same(next) {
                return edge;
            }
            edge = next;
        }
    };
    let start = walk(&|at| at.sub(term, Boundary::Grid, 1));
    let end = walk(&|at| at.add(term, Boundary::Grid, 1));
    Some(Link { uri: link.uri().to_string(), range: start..=end })
}

fn plain<L: EventListener>(term: &Term<L>, point: Point) -> Option<Link> {
    let mut regex = RegexSearch::new(URL).ok()?;
    let (first, last) = (term.line_search_left(point), term.line_search_right(point));
    let found = RegexIter::new(first, last, Direction::Right, term, &mut regex).find(|found| found.contains(&point))?;
    let grid = term.grid();
    let (start, mut end) = (*found.start(), *found.end());
    while end > start && TRAILING.contains(grid[end].c) {
        end = end.sub(term, Boundary::Grid, 1);
    }
    if point > end {
        return None;
    }

    let mut uri = String::new();
    let mut at = start;
    loop {
        if !grid[at].flags.contains(Flags::WIDE_CHAR_SPACER) {
            uri.push(grid[at].c);
        }
        if at == end {
            break;
        }
        at = at.add(term, Boundary::Grid, 1);
    }
    Some(Link { uri, range: start..=end })
}

pub fn allowed(uri: &str) -> bool {
    uri.split_once(':').is_some_and(|(scheme, _)| SCHEMES.iter().any(|allowed| scheme.eq_ignore_ascii_case(allowed)))
}

pub fn open(uri: &str) {
    if !allowed(uri) {
        return;
    }
    if let Ok(mut child) = Command::new("xdg-open").arg(uri).spawn() {
        std::thread::spawn(move || child.wait());
    }
}

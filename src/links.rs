use std::path::{Path, PathBuf};
use std::process::Command;

use alacritty_terminal::event::EventListener;
use alacritty_terminal::index::{Boundary, Direction, Point};
use alacritty_terminal::term::Term;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::search::{Match, RegexIter, RegexSearch};

const SCHEMES: [&str; 4] = ["http", "https", "mailto", "file"];
const URL: &str = r#"(https?://|mailto:|file:)[^\x00-\x1F\x7F-\x9F<>"\s{}\^`]+"#;
// A closing round bracket stays when the URL opens one, as Wikipedia links do.
const TRAILING: &str = ".,;?";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub uri: String,
    pub range: Match,
}

pub fn at<L: EventListener>(term: &Term<L>, point: Point) -> Option<Link> {
    at_in(term, point, None, None)
}

pub fn at_in<L: EventListener>(term: &Term<L>, point: Point, cwd: Option<&str>, home: Option<&str>) -> Option<Link> {
    hyperlink(term, point)
        .or_else(|| plain(term, point))
        .or_else(|| local_path(term, point, cwd, home))
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
    while end > start && (TRAILING.contains(grid[end].c) || (grid[end].c == ')' && unclosed(term, start, end))) {
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

/// Whether the range ends with a closing round bracket that opens nothing.
fn unclosed<L: EventListener>(term: &Term<L>, start: Point, end: Point) -> bool {
    let grid = term.grid();
    let (mut opens, mut closes) = (0, 0);
    let mut at = start;
    loop {
        match grid[at].c {
            '(' => opens += 1,
            ')' => closes += 1,
            _ => {}
        }
        if at == end {
            break;
        }
        at = at.add(term, Boundary::Grid, 1);
    }
    closes > opens
}

fn is_path_delim(ch: char) -> bool {
    ch == '\0' || ch.is_whitespace() || "<>\"'()[]{}".contains(ch)
}

fn local_path<L: EventListener>(term: &Term<L>, point: Point, cwd: Option<&str>, home: Option<&str>) -> Option<Link> {
    let grid = term.grid();
    if is_path_delim(grid[point].c) {
        return None;
    }
    let (first, last) = (term.line_search_left(point), term.line_search_right(point));

    let mut start = point;
    while start > first {
        let prev = start.sub(term, Boundary::Grid, 1);
        if prev == start || is_path_delim(grid[prev].c) {
            break;
        }
        start = prev;
    }

    let mut end = point;
    while end < last {
        let next = end.add(term, Boundary::Grid, 1);
        if next == end || is_path_delim(grid[next].c) {
            break;
        }
        end = next;
    }

    let mut chars: Vec<(Point, char)> = Vec::new();
    let mut at = start;
    loop {
        if !grid[at].flags.contains(Flags::WIDE_CHAR_SPACER) {
            chars.push((at, grid[at].c));
        }
        if at == end {
            break;
        }
        at = at.add(term, Boundary::Grid, 1);
    }

    while chars.len() > 1 && (TRAILING.contains(chars.last()?.1) || chars.last()?.1 == ':') {
        chars.pop();
    }
    for _ in 0..2 {
        if let Some(colon) = chars.iter().rposition(|(_, ch)| *ch == ':')
            && colon > 0
            && colon + 1 < chars.len()
            && chars[colon + 1..].iter().all(|(_, ch)| ch.is_ascii_digit())
        {
            chars.truncate(colon);
        } else {
            break;
        }
    }
    while chars.len() > 1 && (TRAILING.contains(chars.last()?.1) || chars.last()?.1 == ':') {
        chars.pop();
    }

    let path_end = chars.last()?.0;
    if point > path_end {
        return None;
    }
    let candidate: String = chars.iter().map(|(_, ch)| *ch).collect();
    let resolved = if candidate.starts_with('/') {
        PathBuf::from(&candidate)
    } else if let Some(rest) = candidate.strip_prefix("~/") {
        Path::new(home?).join(rest)
    } else if candidate.contains('/') {
        Path::new(cwd?).join(&candidate)
    } else {
        return None;
    };
    if !resolved.exists() {
        return None;
    }
    let target = std::fs::canonicalize(&resolved).unwrap_or(resolved);
    Some(Link { uri: format!("file://{}", target.display()), range: start..=path_end })
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

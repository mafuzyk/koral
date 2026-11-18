use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Boundary, Column, Direction, Line, Point, Side};
use alacritty_terminal::term::Term;
use alacritty_terminal::term::search::{Match, RegexIter, RegexSearch};

/// The query is plain text, not a regex.
pub fn escape(query: &str) -> String {
    let mut escaped = String::with_capacity(query.len());
    for ch in query.chars() {
        if r"\.+*?()|[]{}^$".contains(ch) {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

// shortcut: `current` does not follow lines that scroll while the bar is open, the next step finds its place again.
#[derive(Default)]
pub struct Search {
    pub query: String,
    pub current: Option<Match>,
    regex: Option<RegexSearch>,
}

impl Search {
    pub fn set_query(&mut self, query: String) {
        self.regex = RegexSearch::new(&escape(&query)).ok().filter(|_| !query.is_empty());
        self.query = query;
        self.current = None;
    }

    pub fn step<L: EventListener>(&mut self, term: &Term<L>, direction: Direction) -> Option<Match> {
        let regex = self.regex.as_mut()?;
        let bottom = Point::new(term.bottommost_line(), term.last_column());
        let origin = match (&self.current, direction) {
            (None, _) => bottom,
            (Some(found), Direction::Right) => found.end().add(term, Boundary::None, 1),
            (Some(found), Direction::Left) => found.start().sub(term, Boundary::None, 1),
        };
        self.current = term.search_next(regex, origin, direction, Side::Left, None);
        self.current.clone()
    }

    pub fn visible<L: EventListener>(&mut self, term: &Term<L>, first: Line, last: Line) -> Vec<Match> {
        let Some(regex) = self.regex.as_mut() else {
            return Vec::new();
        };
        let start = Point::new(first, Column(0));
        let end = Point::new(last, term.last_column());
        RegexIter::new(start, end, Direction::Right, term, regex).collect()
    }
}

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
    pub position: Option<(usize, usize)>,
    pub scope: Option<(Line, Line)>,
    regex: Option<RegexSearch>,
}

impl Search {
    pub fn set_query(&mut self, query: String) {
        self.regex = RegexSearch::new(&escape(&query))
            .ok()
            .filter(|_| !query.is_empty());
        self.query = query;
        self.current = None;
        self.position = None;
    }

    pub fn set_scope(&mut self, scope: Option<(Line, Line)>) {
        self.scope = scope;
        self.current = None;
        self.position = None;
    }

    pub fn step<L: EventListener>(
        &mut self,
        term: &Term<L>,
        direction: Direction,
    ) -> Option<Match> {
        let regex = self.regex.as_mut()?;
        if let Some((scope_top, scope_bottom)) = self.scope {
            let start = Point::new(scope_top, Column(0));
            let end = Point::new(scope_bottom, term.last_column());
            let matches: Vec<Match> =
                RegexIter::new(start, end, Direction::Right, term, regex).collect();
            if matches.is_empty() {
                self.current = None;
                self.position = None;
                return None;
            }
            let idx = match &self.current {
                None => match direction {
                    Direction::Left => matches.len() - 1,
                    Direction::Right => 0,
                },
                Some(current) => {
                    let cur = matches.iter().position(|m| m.start() == current.start());
                    match (cur, direction) {
                        (Some(i), Direction::Right) => (i + 1) % matches.len(),
                        (Some(i), Direction::Left) => {
                            if i == 0 {
                                matches.len() - 1
                            } else {
                                i - 1
                            }
                        }
                        (None, Direction::Left) => matches.len() - 1,
                        (None, Direction::Right) => 0,
                    }
                }
            };
            self.current = Some(matches[idx].clone());
            self.position = Some((idx + 1, matches.len()));
            return self.current.clone();
        }

        let bottom = Point::new(term.bottommost_line(), term.last_column());
        let origin = match (&self.current, direction) {
            (None, _) => bottom,
            (Some(found), Direction::Right) => found.end().add(term, Boundary::None, 1),
            (Some(found), Direction::Left) => found.start().sub(term, Boundary::None, 1),
        };
        self.current = term.search_next(regex, origin, direction, Side::Left, None);
        self.position = self.current.as_ref().and_then(|current| {
            let top = Point::new(term.topmost_line(), Column(0));
            let mut total = 0;
            let mut index = None;
            for found in RegexIter::new(top, bottom, Direction::Right, term, regex) {
                total += 1;
                if found.start() == current.start() {
                    index = Some(total);
                }
            }
            Some((index?, total))
        });
        self.current.clone()
    }

    pub fn visible<L: EventListener>(
        &mut self,
        term: &Term<L>,
        first: Line,
        last: Line,
    ) -> Vec<Match> {
        let Some(regex) = self.regex.as_mut() else {
            return Vec::new();
        };
        let (first, last) = match self.scope {
            Some((scope_top, scope_bottom)) => {
                if last < scope_top || first > scope_bottom {
                    return Vec::new();
                }
                (first.max(scope_top), last.min(scope_bottom))
            }
            None => (first, last),
        };
        let start = Point::new(first, Column(0));
        let end = Point::new(last, term.last_column());
        RegexIter::new(start, end, Direction::Right, term, regex).collect()
    }
}

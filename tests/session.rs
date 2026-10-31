use std::time::Instant;

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use koral::session::Session;
use koral::view::{Row, output_text, pinned, rows};

const PROMPT: &str = "\x1b]133;A\x1b\\$ \x1b]133;B\x1b\\";

struct Fixture {
    session: Session<VoidListener>,
    now: Instant,
}

impl Fixture {
    fn new(columns: usize, lines: usize) -> Self {
        let config = Config { scrolling_history: 100, ..Config::default() };
        let term = Term::new(config, &TermSize::new(columns, lines), VoidListener);
        Self { session: Session::new(term), now: Instant::now() }
    }

    fn feed(&mut self, text: &str) {
        self.session.feed(text.as_bytes(), self.now);
    }

    fn command(&mut self, name: &str, output: &[&str], status: i32) {
        self.start(name);
        for line in output {
            self.feed(line);
            self.feed("\r\n");
        }
        self.feed(&format!("\x1b]133;D;{status}\x1b\\"));
    }

    fn start(&mut self, name: &str) {
        self.feed(PROMPT);
        self.feed(name);
        self.feed("\r\n");
        self.feed("\x1b]133;C\x1b\\");
    }

    fn text(&self, line: Line) -> String {
        let grid = self.session.term.grid();
        let row = &grid[line];
        (0..grid.columns()).map(|column| row[Column(column)].c).collect::<String>().trim_end().to_string()
    }

    fn rows(&self) -> Vec<Row> {
        rows(&self.session.term, &self.session.blocks)
    }

    fn visible(&self) -> Vec<String> {
        let mut text: Vec<String> = self.rows().iter().map(|row| self.text(row.line)).collect();
        while text.last().is_some_and(String::is_empty) {
            text.pop();
        }
        text
    }

    fn headers(&self) -> Vec<String> {
        self.rows().iter().filter(|row| row.block.is_some()).map(|row| self.text(row.line)).collect()
    }

    fn fold_latest_finished(&mut self) {
        let id = self.rows().iter().filter_map(|row| row.block).find(|id| {
            self.session.blocks.get(*id).is_some_and(|block| block.status.is_some())
        });
        self.session.blocks.toggle_fold(id.unwrap());
    }
}

#[test]
fn marks_never_reach_the_screen() {
    let mut f = Fixture::new(20, 5);
    f.command("ls", &["a"], 0);
    assert_eq!(f.visible(), vec!["$ ls", "a"]);
}

#[test]
fn a_mark_split_across_reads_still_opens_a_block() {
    let mut f = Fixture::new(20, 5);
    let bytes = PROMPT.as_bytes();
    let (head, tail) = bytes.split_at(bytes.len() - 3);
    f.session.feed(head, f.now);
    f.session.feed(tail, f.now);
    f.feed("ls");
    assert_eq!(f.headers(), vec!["$ ls"]);
}

#[test]
fn prompt_lines_become_headers() {
    let mut f = Fixture::new(20, 10);
    f.command("one", &["a", "b"], 0);
    f.command("two", &["c"], 1);
    assert_eq!(f.headers(), vec!["$ one", "$ two"]);
    assert_eq!(f.visible(), vec!["$ one", "a", "b", "$ two", "c"]);
}

#[test]
fn folding_hides_only_that_blocks_output() {
    let mut f = Fixture::new(20, 10);
    f.command("one", &["a", "b"], 0);
    f.command("two", &["c"], 0);
    f.fold_latest_finished();

    assert_eq!(f.visible(), vec!["$ one", "$ two", "c"]);
    assert_eq!(f.rows()[0].hidden, 2);
}

#[test]
fn folding_keeps_a_wrapped_command_line_visible() {
    let mut f = Fixture::new(8, 10);
    f.command("a-long-command", &["out"], 0);
    f.command("two", &[], 0);
    f.fold_latest_finished();

    assert_eq!(f.visible(), vec!["$ a-long", "-command", "$ two"]);
}

#[test]
fn folds_survive_a_resize() {
    let mut f = Fixture::new(20, 10);
    f.command("one", &["output that wraps when narrow"], 0);
    f.command("two", &[], 0);
    f.fold_latest_finished();
    f.session.term.resize(TermSize::new(10, 10));

    assert_eq!(f.headers(), vec!["$ one", "$ two"]);
    assert_eq!(f.visible(), vec!["$ one", "$ two"]);
}

#[test]
fn an_abandoned_prompt_is_not_a_header() {
    let mut f = Fixture::new(20, 10);
    f.feed(PROMPT);
    f.feed("\r\n");
    f.feed(PROMPT);
    f.feed("ls");
    assert_eq!(f.headers(), vec!["$ ls"]);
}

#[test]
fn redrawing_the_prompt_on_each_key_keeps_one_anchor() {
    let mut f = Fixture::new(20, 10);
    f.feed(PROMPT);
    for typed in ["l", "ls", "ls ", "ls -"] {
        f.feed(&format!("\r$ \x1b]133;B\x1b\\{typed}\x1b[K"));
    }
    assert_eq!(f.headers(), vec!["$ ls -"]);
    let line = f.rows()[0].line;
    let cell = &f.session.term.grid()[line][Column(0)];
    assert_eq!(cell.zerowidth().map(<[char]>::len), Some(1));
}

#[test]
fn a_wrapped_prompt_is_anchored_on_its_first_row() {
    let mut f = Fixture::new(8, 10);
    f.feed("\x1b]133;A\x1b\\a-very-long-prompt \x1b]133;B\x1b\\");
    assert_eq!(f.headers(), vec!["a-very-l"]);
}

#[test]
fn marks_in_the_alternate_screen_are_ignored() {
    let mut f = Fixture::new(20, 5);
    f.feed("\x1b[?1049h");
    f.feed(PROMPT);
    assert!(f.session.blocks.is_empty());
}

#[test]
fn clearing_scrollback_leaves_a_consistent_view() {
    let mut f = Fixture::new(20, 3);
    f.command("one", &["a", "b", "c", "d"], 0);
    f.feed("\x1b[H\x1b[2J\x1b[3J");
    f.feed(PROMPT);
    f.feed("two");
    assert_eq!(f.headers(), vec!["$ two"]);
}

#[test]
fn output_text_is_only_that_commands_output() {
    let mut f = Fixture::new(20, 10);
    f.command("one", &["a", "b"], 0);
    f.command("two", &["c"], 0);
    let id = f.rows()[0].block.unwrap();
    assert_eq!(output_text(&f.session.term, &f.session.blocks, id).as_deref(), Some("a\nb"));
}

#[test]
fn output_text_joins_wrapped_lines_and_skips_a_wrapped_command() {
    let mut f = Fixture::new(8, 10);
    f.command("a-long-command", &["0123456789ab"], 0);
    f.command("two", &[], 0);
    let id = f.rows()[0].block.unwrap();
    assert_eq!(output_text(&f.session.term, &f.session.blocks, id).as_deref(), Some("0123456789ab"));
}

#[test]
fn output_text_never_contains_an_anchor() {
    let mut f = Fixture::new(20, 10);
    f.command("one", &["a"], 0);
    f.command("two", &["b"], 0);
    let ids: Vec<u32> = f.rows().iter().filter_map(|row| row.block).collect();
    for id in ids {
        let text = output_text(&f.session.term, &f.session.blocks, id).unwrap();
        assert!(text.chars().all(|ch| (ch as u32) < 0x10_0000), "{text:?}");
    }
}

#[test]
fn output_text_of_a_running_command_stops_at_the_cursor() {
    let mut f = Fixture::new(20, 10);
    f.start("sleep");
    f.feed("partial");
    let id = f.rows()[0].block.unwrap();
    assert_eq!(output_text(&f.session.term, &f.session.blocks, id).as_deref(), Some("partial"));
}

#[test]
fn output_text_of_an_unknown_block_is_none() {
    let f = Fixture::new(20, 5);
    assert_eq!(output_text(&f.session.term, &f.session.blocks, 9), None);
}

#[test]
fn a_running_command_is_pinned_once_its_header_scrolls_away() {
    let mut f = Fixture::new(20, 4);
    f.start("build");
    for _ in 0..10 {
        f.feed("line\r\n");
    }
    let rows = f.rows();
    let id = rows[0].block;
    assert!(id.is_some());
    assert_eq!(pinned(&rows, 3, &f.session.blocks), id);
    assert_eq!(pinned(&rows, 0, &f.session.blocks), None);
}

#[test]
fn a_finished_command_is_not_pinned() {
    let mut f = Fixture::new(20, 4);
    f.command("build", &["a", "b", "c"], 0);
    let rows = f.rows();
    assert_eq!(pinned(&rows, 2, &f.session.blocks), None);
}

mod navigation {
    use alacritty_terminal::index::Line;
    use koral::blocks::BlockStore;
    use koral::marks::Mark;
    use koral::view::{Row, fold_target, next_prompt, previous_prompt};
    use std::time::Instant;

    fn row(block: Option<u32>) -> Row {
        Row { line: Line(0), block, hidden: 0 }
    }

    fn sample() -> Vec<Row> {
        vec![row(Some(0)), row(None), row(None), row(Some(1)), row(None), row(Some(2))]
    }

    #[test]
    fn previous_prompt_is_strictly_above_the_top() {
        assert_eq!(previous_prompt(&sample(), 3), Some(0));
        assert_eq!(previous_prompt(&sample(), 4), Some(3));
        assert_eq!(previous_prompt(&sample(), 0), None);
    }

    #[test]
    fn next_prompt_is_strictly_below_the_top() {
        assert_eq!(next_prompt(&sample(), 0), Some(3));
        assert_eq!(next_prompt(&sample(), 3), Some(5));
        assert_eq!(next_prompt(&sample(), 5), None);
    }

    #[test]
    fn fold_target_is_the_last_finished_block_on_screen() {
        let mut blocks = BlockStore::default();
        let now = Instant::now();
        for status in [Some(0), Some(0)] {
            blocks.apply(Mark::PromptStart, now);
            blocks.apply(Mark::PromptEnd, now);
            blocks.apply(Mark::CommandStart(None), now);
            blocks.apply(Mark::CommandEnd(status), now);
        }
        blocks.apply(Mark::PromptStart, now);
        blocks.apply(Mark::PromptEnd, now);

        assert_eq!(fold_target(&sample(), sample().len(), &blocks), Some(1));
        assert_eq!(fold_target(&sample(), 2, &blocks), Some(0));
    }
}

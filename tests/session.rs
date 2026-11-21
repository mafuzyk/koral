use std::time::Instant;

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term, TermMode};
use koral::session::Session;
use koral::view::{Placed, Row, output_text, pinned, placed, rows};

const PROMPT: &str = "\x1b]133;A\x1b\\$ \x1b]133;B\x1b\\";
const TALL_PROMPT: &str = "\x1b]133;A\x1b\\\r\ninfo\r\n$ \x1b]133;B\x1b\\";

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

    fn tall_command(&mut self, name: &str, output: &[&str]) {
        self.feed(TALL_PROMPT);
        self.feed(&format!("{name}\r\n\x1b]133;C\x1b\\"));
        for line in output {
            self.feed(&format!("{line}\r\n"));
        }
        self.feed("\x1b]133;D;0\x1b\\");
    }

    /// A 1x1 image shown over `columns` by `rows` cells.
    fn show(&mut self, keys: &str) {
        self.feed(&format!("\x1b_Ga=T,f=24,s=1,v=1,q=2,{keys};AAAA\x1b\\"));
    }

    fn pictures(&self) -> Vec<Placed> {
        let rows = self.rows();
        let lines = self.session.term.screen_lines();
        let top = rows.len().saturating_sub(lines);
        placed(&self.session.term, &self.session.graphics, &rows, top, lines)
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
fn folding_stops_before_the_next_prompt_of_several_lines() {
    let mut f = Fixture::new(20, 12);
    f.tall_command("one", &["a", "b"]);
    f.tall_command("two", &["c"]);
    f.fold_latest_finished();

    assert_eq!(f.visible(), vec!["", "info", "$ one", "", "info", "$ two", "c"]);
    assert_eq!(f.rows()[2].hidden, 2);
}

#[test]
fn a_tall_prompt_is_measured_even_with_the_scrollback_full() {
    let mut f = Fixture::new(20, 4);
    for _ in 0..40 {
        f.tall_command("one", &["a"]);
    }
    let header = f.rows().iter().position(|row| row.block.is_some()).unwrap();
    f.fold_latest_finished();

    assert_eq!(f.rows()[header].hidden, 1);
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
fn a_prompt_after_a_full_output_line_is_its_own_header() {
    let mut f = Fixture::new(10, 6);
    f.start("printf");
    f.feed("0123456789");
    f.feed("\x1b]133;D;0\x1b\\");
    f.feed(PROMPT);
    f.feed("ls\r\n");

    assert_eq!(f.headers(), vec!["$ printf", "$ ls"]);
}

#[test]
fn a_reflowed_prompt_stays_out_of_the_command_before() {
    let mut f = Fixture::new(20, 12);
    f.command("one", &["a", "b"], 0);
    f.feed("\x1b]133;A\x1b\\head\r\nxxxxxx\r\n$ \x1b]133;B\x1b\\");
    f.feed("two\r\n\x1b]133;C\x1b\\");
    f.feed("c\r\n\x1b]133;D;0\x1b\\");
    let id = f.rows()[0].block.unwrap();
    f.session.term.resize(TermSize::new(5, 12));
    f.feed("\x1b]133;B\x1b\\");

    assert_eq!(output_text(&f.session.term, &f.session.blocks, id).as_deref(), Some("a\nb"));
}

#[test]
fn marks_in_the_alternate_screen_are_ignored() {
    let mut f = Fixture::new(20, 5);
    f.feed("\x1b[?1049h");
    f.feed(PROMPT);
    assert!(f.session.blocks.is_empty());
}

#[test]
fn a_command_end_on_the_alternate_screen_finishes_the_block() {
    let mut f = Fixture::new(20, 6);
    f.start("vim");
    let id = f.rows()[0].block.unwrap();
    f.feed("\x1b[?1049h");
    f.feed("\x1b]133;D;0\x1b\\");
    f.feed("\x1b[?1049l");
    f.feed(PROMPT);
    f.feed("ls\r\n");

    let block = f.session.blocks.get(id).unwrap();
    assert_eq!(block.status, Some(0));
    assert!(block.duration.is_some());
    assert_eq!(f.headers(), vec!["$ vim", "$ ls"]);
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
fn output_text_leaves_out_the_next_prompt_of_several_lines() {
    let mut f = Fixture::new(20, 12);
    f.tall_command("one", &["a", "b"]);
    f.tall_command("two", &["c"]);
    let id = f.rows()[2].block.unwrap();
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

#[test]
fn an_image_command_never_reaches_the_screen() {
    let mut f = Fixture::new(20, 5);
    f.show("i=1,c=2,r=2");
    assert_eq!(f.visible(), Vec::<String>::new());
}

#[test]
fn an_image_sits_on_the_cursor_cell() {
    let mut f = Fixture::new(20, 10);
    f.feed("ab\r\ncd");
    f.show("i=1,c=3,r=2");
    let picture = f.pictures()[0];
    assert_eq!((picture.row, picture.column, picture.columns, picture.rows), (1, 2, 3, 2));
}

/// icat centres an image by moving the cursor forward before the command, so the placement must
/// land on the column the client chose instead of a centred one of the terminal's own making.
#[test]
fn an_image_lands_on_the_column_the_client_moved_the_cursor_to() {
    let mut f = Fixture::new(80, 10);
    f.feed("\x1b[28C");
    f.show("i=1,c=24,r=6");
    let picture = f.pictures()[0];
    assert_eq!((picture.row, picture.column), (0, 28));
}

/// OpenTUI wraps a repaint in a synchronized update: it moves the cursor to the image cell, sends
/// the placement, puts the cursor back and ends the update. vte replays those moves only at the
/// ESU, so the image must not land where the cursor sat when the batch began.
#[test]
fn an_image_in_a_synchronized_update_lands_on_the_cell_the_batch_names() {
    let batch = "\x1b[?2026h\x1b[5;4H\x1b_Ga=p,i=1,c=2,r=2,C=1,q=2\x1b\\\x1b[1;9H\x1b[?2026l";
    // Whole in one read, and split inside the image command as a chunked client sends it.
    for split in [batch.len(), 30] {
        let mut f = Fixture::new(20, 10);
        f.feed("\x1b[?1049h");
        f.feed("\x1b_Ga=t,f=24,s=1,v=1,i=1,q=2;AAAA\x1b\\");
        f.feed("$ prompt");
        let (head, tail) = batch.split_at(split);
        f.session.feed(head.as_bytes(), f.now);
        f.session.feed(tail.as_bytes(), f.now);

        let cursor = f.session.term.grid().cursor.point;
        assert_eq!((cursor.line, cursor.column), (Line(0), Column(8)), "the cursor goes back to the prompt");
        let picture = f.pictures()[0];
        assert_eq!((picture.row, picture.column), (4, 3), "the image sits on the cell the batch named");
    }
}

/// A client that paints on the alternate screen leaves it and places on the primary one in the
/// same read. The exit has to be reconciled before the placement, and the end-of-feed cleanup
/// must not then drop the image that just landed on the primary screen.
#[test]
fn an_image_placed_after_leaving_the_alternate_screen_stays() {
    for batch in [
        "\x1b[?2026h\x1b[5;4H\x1b_Ga=p,i=1,c=2,r=2,C=1,q=2\x1b\\\x1b[1;9H\x1b[?2026l",
        "\x1b[5;4H\x1b_Ga=p,i=1,c=2,r=2,C=1,q=2\x1b\\\x1b[1;9H",
    ] {
        let mut f = Fixture::new(20, 10);
        f.feed("\x1b[?1049h");
        f.feed("\x1b_Ga=t,f=24,s=1,v=1,i=1,q=2;AAAA\x1b\\");
        f.feed("$ prompt");
        f.feed(&format!("\x1b[?1049l{batch}"));

        let pictures = f.pictures();
        assert_eq!(pictures.len(), 1, "the image on the primary screen outlived the exit");
        assert_eq!((pictures[0].row, pictures[0].column), (4, 3), "it sits on the cell the batch named");
        let cursor = f.session.term.grid().cursor.point;
        assert_eq!((cursor.line, cursor.column), (Line(0), Column(8)), "the cursor stays on the primary screen");
    }
}

/// The reopened update must still end at the client's ESU: whatever follows the batch applies
/// at once instead of waiting for a timeout.
#[test]
fn output_after_the_end_of_a_synchronized_update_applies_at_once() {
    let mut f = Fixture::new(20, 10);
    f.feed("\x1b[?1049h");
    f.feed("\x1b_Ga=t,f=24,s=1,v=1,i=1,q=2;AAAA\x1b\\");
    f.feed("$ ");
    f.feed("\x1b[?2026h\x1b[5;4H\x1b_Ga=p,i=1,c=2,r=2,C=1,q=2\x1b\\\x1b[1;3H\x1b[?2026l");
    f.feed("x");

    assert_eq!(f.text(Line(0)), "$ x");
    let cursor = f.session.term.grid().cursor.point;
    assert_eq!((cursor.line, cursor.column), (Line(0), Column(3)), "the update ended with its ESU");
}

/// A client that clears a preview inside a synchronized update names the cell with a move, and
/// that move is only in the parser until the update ends, so the delete has to wait for it too.
#[test]
fn a_delete_in_a_synchronized_update_follows_the_cell_the_batch_names() {
    let mut f = Fixture::new(20, 10);
    f.feed("\x1b_Ga=t,f=24,s=1,v=1,i=1,q=2;AAAA\x1b\\");
    f.feed("\x1b_Ga=t,f=24,s=1,v=1,i=2,q=2;AAAA\x1b\\");
    f.feed("\x1b[H\x1b_Ga=p,i=1,c=2,r=2,C=1,q=2\x1b\\");
    f.feed("\x1b[5;1H\x1b_Ga=p,i=2,c=2,r=2,C=1,q=2\x1b\\");
    f.feed("\x1b[1;1H");
    f.feed("\x1b[?2026h\x1b[5;1H\x1b_Ga=d,d=c\x1b\\\x1b[1;1H\x1b[?2026l");

    let pictures = f.pictures();
    assert_eq!(pictures.len(), 1);
    assert_eq!(pictures[0].row, 0, "the image the batch pointed at went");
}

#[test]
fn an_image_is_not_centred_when_the_cursor_sits_at_the_left_edge() {
    let mut f = Fixture::new(80, 10);
    f.show("i=1,c=2,r=2");
    assert_eq!(f.pictures()[0].column, 0);
}

#[test]
fn an_image_after_a_wide_character_takes_the_column_the_cursor_reached() {
    let mut f = Fixture::new(20, 10);
    f.feed("日");
    f.show("i=1,c=2,r=2");
    assert_eq!(f.pictures()[0].column, 2);
}

#[test]
fn an_unsupported_transfer_mode_is_answered_instead_of_dropped() {
    let mut f = Fixture::new(20, 5);
    f.feed("\x1b_Ga=T,f=100,t=s,s=1,v=1,S=19,i=4;L2ljYXQtSlRWTU5HNU9ORklJQw\x1b\\");
    assert!(f.session.replies.iter().any(|reply| reply.contains("ENOTSUPPORTED")), "{:?}", f.session.replies);
    assert!(f.pictures().is_empty());
}

#[test]
fn a_placement_carries_its_crop_to_the_view() {
    let mut f = Fixture::new(20, 10);
    f.feed("\x1b_Ga=t,f=24,s=2,v=2,i=1,q=2;AAAAAAAAAAAAAAAA\x1b\\");
    f.feed("\x1b_Ga=p,i=1,x=1,y=0,w=1,h=2,c=1,r=1,C=1,q=2\x1b\\");
    assert_eq!(f.pictures()[0].crop, [1, 0, 1, 2]);
}

#[test]
fn the_current_style_can_be_read_back() {
    let mut f = Fixture::new(20, 10);
    f.feed("\x1b[1;4:3m\x1bP$qm\x1b\\\x1b[m");
    assert_eq!(f.session.replies, vec!["\x1bP1$r0;1;4:3m\x1b\\"]);
}

#[test]
fn a_plain_style_reads_back_as_zero() {
    let mut f = Fixture::new(20, 10);
    f.feed("\x1bP$qm\x1b\\");
    assert_eq!(f.session.replies, vec!["\x1bP1$r0m\x1b\\"]);
}

#[test]
fn a_setting_koral_does_not_report_is_refused() {
    let mut f = Fixture::new(20, 10);
    f.feed("\x1bP$q q\x1b\\");
    assert_eq!(f.session.replies, vec!["\x1bP0$r\x1b\\"]);
}

#[test]
fn an_image_moves_the_cursor_past_itself() {
    let mut f = Fixture::new(20, 10);
    f.show("i=1,c=3,r=4");
    let cursor = f.session.term.grid().cursor.point;
    assert_eq!((cursor.line, cursor.column), (Line(3), Column(3)));
}

#[test]
fn the_cursor_key_leaves_the_cursor_alone() {
    let mut f = Fixture::new(20, 10);
    f.show("i=1,c=3,r=4,C=1");
    let cursor = f.session.term.grid().cursor.point;
    assert_eq!((cursor.line, cursor.column), (Line(0), Column(0)));
}

#[test]
fn an_image_scrolls_with_the_text() {
    let mut f = Fixture::new(20, 5);
    f.show("i=1,c=2,r=2");
    f.feed("\r\na\r\nb\r\nc\r\nd");
    assert_eq!(f.pictures()[0].row, -1);
}

#[test]
fn an_image_scrolled_past_its_height_is_not_listed() {
    let mut f = Fixture::new(20, 5);
    f.show("i=1,c=2,r=2");
    f.feed("\r\n\r\n\r\n\r\n\r\n\r\n\r\n\r\n");
    assert!(f.pictures().is_empty());
}

#[test]
fn a_folded_command_hides_its_image() {
    let mut f = Fixture::new(20, 12);
    f.start("one");
    f.show("i=1,c=2,r=2");
    f.feed("\r\n\x1b]133;D;0\x1b\\");
    f.command("two", &["c"], 0);
    assert_eq!(f.pictures().len(), 1);
    f.fold_latest_finished();
    assert!(f.pictures().is_empty());
}

/// A modal draws a hole for a picture that sits under the text: the spaces it paints with the
/// default background are not an erase, so they must leave the image on its cell.
#[test]
fn text_painted_over_an_image_leaves_it_on_screen() {
    for paint in ["  ", "ab", "\x1b[48;2;12;12;18m  \x1b[49m"] {
        let mut f = Fixture::new(20, 10);
        f.feed("\x1b[2;3H");
        f.show("i=1,p=1,c=2,r=2,C=1,z=-1073741822");
        f.feed("\x1b[2;3H");
        f.feed(paint);
        let pictures = f.pictures();
        assert_eq!(pictures.len(), 1, "the image outlives the paint over its cell");
        assert_eq!((pictures[0].row, pictures[0].column), (1, 2), "it stays on the cell it was placed on");
    }
}

/// Output scrolled the image away from the cell it was placed on; the anchor travelled with the
/// text, and paint over the new cell must leave it there instead of on the old one.
#[test]
fn text_painted_over_a_scrolled_image_keeps_its_new_cell() {
    let mut f = Fixture::new(20, 5);
    f.feed("\x1b[3;1H");
    f.show("i=1,p=1,c=2,r=2,C=1");
    f.feed("\x1b[5;1H\n");
    assert_eq!(f.pictures()[0].row, 1, "the image moved up with the text");
    f.feed("\x1b[2;1H  ");
    let pictures = f.pictures();
    assert_eq!(pictures.len(), 1, "one anchor, put back once");
    assert_eq!((pictures[0].row, pictures[0].column), (1, 0), "it keeps the cell the scroll gave it");
}

/// ED 3 clears the scrollback, not the screen: the visible image and its anchor stay.
#[test]
fn clearing_the_scrollback_keeps_the_image_on_screen() {
    let mut f = Fixture::new(20, 5);
    f.feed(&"\r\n".repeat(120));
    f.feed("\x1b[5;1H");
    f.show("i=1,c=2,r=2,C=1");
    f.feed("\x1b[3J");
    f.feed("\x1b[5;1H  ");
    let pictures = f.pictures();
    assert_eq!(pictures.len(), 1, "ED 3 left the visible cell alone");
    assert_eq!((pictures[0].row, pictures[0].column), (4, 0));
}

/// The alternate screen scrolls inside a region; the anchor rides the cells and must not be
/// duplicated when text is painted over its new cell.
#[test]
fn a_scroll_in_a_region_keeps_one_anchor() {
    let mut f = Fixture::new(20, 10);
    f.feed("\x1b[?1049h\x1b[1;5r\x1b[3;1H");
    f.show("i=1,p=1,c=2,r=2,C=1");
    f.feed("\x1b[5;1H\n");
    assert_eq!(f.pictures()[0].row, 1, "the image moved up inside the region");
    f.feed("\x1b[2;1H  ");
    let pictures = f.pictures();
    assert_eq!(pictures.len(), 1, "one anchor after the region scrolled");
    assert_eq!((pictures[0].row, pictures[0].column), (1, 0));
}

/// At a full scrollback the grid recycles lines, but the anchor still moves with the cell it is on.
#[test]
fn a_scroll_at_the_full_history_keeps_one_anchor() {
    let mut f = Fixture::new(20, 5);
    f.feed(&"\r\n".repeat(120));
    f.feed("\x1b[5;1H");
    f.show("i=1,p=1,c=2,r=2,C=1");
    f.feed("\n");
    assert_eq!(f.pictures()[0].row, 3, "the image moved up at full history");
    f.feed("\x1b[4;1H  ");
    let pictures = f.pictures();
    assert_eq!(pictures.len(), 1, "one anchor at full history");
    assert_eq!((pictures[0].row, pictures[0].column), (3, 0));
}

/// A wide glyph written over the anchor cell keeps the picture underneath it.
#[test]
fn a_wide_character_over_an_image_keeps_it() {
    let mut f = Fixture::new(20, 10);
    f.show("i=1,p=1,c=2,r=2,C=1");
    f.feed("日");
    let pictures = f.pictures();
    assert_eq!(pictures.len(), 1, "the wide glyph replaced the text, not the picture");
    assert_eq!((pictures[0].row, pictures[0].column), (0, 0));
}

#[test]
fn clearing_the_screen_removes_the_image() {
    let mut f = Fixture::new(20, 5);
    f.show("i=1,c=2,r=2");
    f.feed("\x1b[H\x1b[2J");
    assert!(f.pictures().is_empty());
}

/// ED 2 is an erase, unlike text over the picture: it takes the image with the cells.
#[test]
fn clearing_the_screen_removes_an_image_text_was_painted_over() {
    let mut f = Fixture::new(20, 5);
    f.show("i=1,c=2,r=2,C=1");
    f.feed("\x1b[H  ");
    assert_eq!(f.pictures().len(), 1, "the picture survived the text");
    f.feed("\x1b[H\x1b[2J");
    assert!(f.pictures().is_empty(), "the clear took it");
}

#[test]
fn deleting_placements_removes_the_image_from_view() {
    let mut f = Fixture::new(20, 5);
    f.show("i=1,c=2,r=2");
    f.feed("\x1b_Ga=d,d=a\x1b\\");
    assert!(f.pictures().is_empty());
}

/// The chat clears a preview that scrolled out of its box: the cursor names the cell.
#[test]
fn a_delete_at_the_cursor_leaves_the_images_elsewhere() {
    let mut f = Fixture::new(20, 10);
    f.feed("\x1b_Ga=t,f=24,s=1,v=1,i=1,q=2;AAAA\x1b\\");
    f.feed("\x1b_Ga=t,f=24,s=1,v=1,i=2,q=2;AAAA\x1b\\");
    f.feed("\x1b[H\x1b_Ga=p,i=1,c=2,r=2,C=1,q=2\x1b\\");
    f.feed("\x1b[5;1H\x1b_Ga=p,i=2,c=2,r=2,C=1,q=2\x1b\\");
    assert_eq!(f.pictures().len(), 2);
    f.feed("\x1b[5;1H\x1b_Ga=d,d=c\x1b\\");
    let pictures = f.pictures();
    assert_eq!(pictures.len(), 1, "only the one the cursor is on goes");
    assert_eq!(pictures[0].row, 0);
}

#[test]
fn a_delete_at_a_cell_clears_that_cell_and_leaves_the_others() {
    let mut f = Fixture::new(20, 10);
    f.feed("\x1b_Ga=t,f=24,s=1,v=1,i=1,q=2;AAAA\x1b\\");
    f.feed("\x1b_Ga=t,f=24,s=1,v=1,i=2,q=2;AAAA\x1b\\");
    f.feed("\x1b[H\x1b_Ga=p,i=1,c=2,r=2,C=1,q=2\x1b\\");
    f.feed("\x1b[8;1H\x1b_Ga=p,i=2,c=2,r=2,C=1,q=2\x1b\\");
    // The client counts cells from one, and the placement covers its own two cells.
    f.feed("\x1b_Ga=d,d=p,x=2,y=8\x1b\\");
    let pictures = f.pictures();
    assert_eq!(pictures.len(), 1);
    assert_eq!(pictures[0].row, 0);
}

#[test]
fn a_delete_by_row_and_by_column_follow_the_same_cells() {
    let mut f = Fixture::new(20, 10);
    f.feed("\x1b_Ga=t,f=24,s=1,v=1,i=1,q=2;AAAA\x1b\\");
    f.feed("\x1b[H\x1b_Ga=p,i=1,c=2,r=2,C=1,q=2\x1b\\");
    f.feed("\x1b_Ga=d,d=x,x=1\x1b\\");
    assert!(f.pictures().is_empty(), "the column holds the placement");

    let mut f = Fixture::new(20, 10);
    f.feed("\x1b_Ga=t,f=24,s=1,v=1,i=1,q=2;AAAA\x1b\\");
    f.feed("\x1b[H\x1b_Ga=p,i=1,c=2,r=2,C=1,q=2\x1b\\");
    f.feed("\x1b_Ga=d,d=y,y=1\x1b\\");
    assert!(f.pictures().is_empty(), "the row holds the placement");

    let mut f = Fixture::new(20, 10);
    f.feed("\x1b_Ga=t,f=24,s=1,v=1,i=1,q=2;AAAA\x1b\\");
    f.feed("\x1b[H\x1b_Ga=p,i=1,c=2,r=2,C=1,q=2\x1b\\");
    f.feed("\x1b_Ga=d,d=z,z=0\x1b\\");
    assert!(f.pictures().is_empty(), "the z-index matches");
}

/// A thumbnail and a viewer show one transmission in two places, so both must survive.
#[test]
fn one_transmission_shows_on_two_cells() {
    let mut f = Fixture::new(20, 10);
    f.feed("\x1b_Ga=t,f=24,s=2,v=2,i=1,q=2;AAAAAAAAAAAAAAAA\x1b\\");
    f.feed("\x1b[H\x1b_Ga=p,i=1,c=2,r=2,C=1,q=2\x1b\\");
    f.feed("\x1b[4;1H\x1b_Ga=p,i=1,c=2,r=2,C=1,q=2\x1b\\");
    let mut pictures = f.pictures();
    pictures.sort_by_key(|picture| picture.row);
    assert_eq!(pictures.len(), 2);
    assert_eq!((pictures[0].row, pictures[1].row), (0, 3));
}

#[test]
fn leaving_the_alternate_screen_takes_its_images_away() {
    let mut f = Fixture::new(20, 5);
    f.feed("\x1b[?1049h");
    f.show("i=1,c=2,r=2,C=1");
    assert_eq!(f.pictures().len(), 1);
    f.feed("\x1b[?1049l");
    assert!(f.pictures().is_empty());
    assert_eq!(f.session.graphics.tallest(), 0, "the placement outlived the screen");
}

#[test]
fn a_video_repainting_one_cell_keeps_one_anchor() {
    let mut f = Fixture::new(20, 5);
    for _ in 0..100 {
        f.feed("\x1b[H");
        f.show("i=1,c=2,r=2,C=1");
    }
    let cell = &f.session.term.grid()[Line(0)][Column(0)];
    assert_eq!(cell.zerowidth().map_or(0, <[char]>::len), 1);
    assert_eq!(f.pictures().len(), 1);
}

#[test]
fn replies_wait_for_the_terminal_to_send_them() {
    let mut f = Fixture::new(20, 5);
    f.feed("\x1b_Ga=q,f=24,s=1,v=1,i=31;AAAA\x1b\\");
    assert_eq!(f.session.replies, vec!["\x1b_Gi=31;OK\x1b\\"]);
}

#[test]
fn images_also_work_on_the_alternate_screen() {
    let mut f = Fixture::new(20, 5);
    f.feed("\x1b[?1049h");
    f.show("i=1,c=2,r=2,C=1");
    assert_eq!(f.pictures().len(), 1);
}

#[test]
fn text_with_an_image_anchor_is_copied_clean() {
    let mut f = Fixture::new(20, 5);
    f.show("i=1,c=2,r=2,C=1");
    f.feed("hi");
    let line = f.text(Line(0));
    assert_eq!(line, "hi");
}

#[test]
fn a_one_row_video_keeps_one_anchor() {
    let mut f = Fixture::new(20, 5);
    for _ in 0..3 {
        f.feed("\x1b[H");
        f.show("i=1,c=2,r=1,C=1");
    }
    let cell = &f.session.term.grid()[Line(0)][Column(0)];
    assert_eq!(cell.zerowidth().map_or(0, <[char]>::len), 1);
}

#[test]
fn a_placement_moves_the_cursor_unless_it_is_asked_not_to() {
    let mut f = Fixture::new(20, 10);
    f.feed("\x1b_Ga=t,f=24,s=1,v=1,i=1,q=2;AAAA\x1b\\");
    f.feed("\x1b_Ga=p,i=1,c=3,r=2,q=2\x1b\\");
    let moved = f.session.term.grid().cursor.point;
    assert_eq!((moved.line, moved.column), (Line(1), Column(3)));

    let mut f = Fixture::new(20, 10);
    f.feed("\x1b_Ga=t,f=24,s=1,v=1,i=1,q=2;AAAA\x1b\\");
    f.feed("\x1b_Ga=p,i=1,c=3,r=2,C=1,q=2\x1b\\");
    let kept = f.session.term.grid().cursor.point;
    assert_eq!((kept.line, kept.column), (Line(0), Column(0)));
}

#[test]
fn an_image_over_the_foot_of_another_keeps_its_own_anchor() {
    let mut f = Fixture::new(20, 10);
    f.show("i=1,c=2,r=3,C=1");
    f.feed("\x1b[3;1H");
    f.show("i=1,c=2,r=1,C=1");
    assert_eq!(f.pictures().len(), 2);
}

#[test]
fn a_video_without_ids_keeps_one_anchor() {
    let mut f = Fixture::new(20, 5);
    for _ in 0..5 {
        f.feed("\x1b[H\x1b_Ga=T,f=24,s=1,v=1,c=2,r=2,C=1,q=2;AAAA\x1b\\");
    }
    let cell = &f.session.term.grid()[Line(0)][Column(0)];
    assert_eq!(cell.zerowidth().map_or(0, <[char]>::len), 1);
    assert_eq!(f.pictures().len(), 1);
}

#[test]
fn a_frame_without_an_id_leaves_the_store_when_replaced() {
    let mut f = Fixture::new(20, 5);
    let frame = "\x1b[H\x1b_Ga=T,f=24,s=1,v=1,c=2,r=2,C=1,q=2;AAAA\x1b\\";
    f.feed(frame);
    let first = f.pictures()[0].image;
    f.feed(frame);
    assert!(f.session.graphics.image(first).is_none());
}

#[test]
fn named_images_on_one_cell_both_stay() {
    let mut f = Fixture::new(20, 5);
    f.show("i=1,c=2,r=2,C=1");
    f.show("i=2,c=2,r=2,C=1");
    assert_eq!(f.pictures().len(), 2);
}

#[test]
fn an_app_can_turn_the_keyboard_protocol_on() {
    let term = Term::new(koral::pty::term_config(), &TermSize::new(20, 5), VoidListener);
    let mut session = Session::new(term);
    session.feed(b"\x1b[>1u", Instant::now());
    assert!(session.term.mode().contains(TermMode::DISAMBIGUATE_ESC_CODES));
}

#[test]
fn a_word_selection_stops_at_a_space_after_an_anchor() {
    let mut fixture = Fixture::new(20, 5);
    fixture.feed(&format!("{PROMPT}foo bar"));
    let line = fixture.session.term.grid().cursor.point.line;
    let point = Point::new(line, Column(3));
    fixture.session.term.selection = Some(Selection::new(SelectionType::Semantic, point, Side::Left));
    assert_eq!(fixture.session.term.selection_to_string().as_deref(), Some("foo"));
}

#[test]
fn overwriting_a_wide_spacer_over_an_image_keeps_it() {
    let mut f = Fixture::new(20, 10);
    f.show("i=1,p=1,c=2,r=2,C=1");
    f.feed("日");
    f.feed("\x1b[1;2Ha");
    let pictures = f.pictures();
    assert_eq!(pictures.len(), 1, "clear_wide on the right spacer must preserve the picture anchor");
    assert_eq!((pictures[0].row, pictures[0].column), (0, 0));
}

#[test]
fn line_erase_over_an_image_preserves_its_anchor() {
    for seq in ["\x1b[K", "\x1b[2K", "\x1b[2X"] {
        let mut f = Fixture::new(20, 10);
        f.show("i=1,p=1,c=2,r=2,C=1");
        f.feed("\x1b[1;1H");
        f.feed(seq);
        assert_eq!(f.pictures().len(), 1, "sequence {seq:?} should keep the picture anchor");
    }
}

#[test]
fn clearing_the_screen_prunes_orphan_placements() {
    let mut f = Fixture::new(20, 5);
    f.feed("\x1b[?1049h");
    f.show("i=1,c=2,r=4,C=1");
    assert_eq!(f.session.graphics.tallest(), 4);
    f.feed("\x1b[2J");
    assert!(f.pictures().is_empty());
    assert_eq!(f.session.graphics.tallest(), 0, "orphan placements must be pruned after ED 2");
}

#[test]
fn synchronized_marks_and_status_read_flushed_grid_state() {
    let mut f = Fixture::new(20, 6);
    f.feed(&format!("\x1b[?2026h\r\n\r\n{PROMPT}\x1b[1m\x1bP$qm\x1b\\\x1b[?2026l"));
    let block_row = f.rows().into_iter().find(|row| row.block.is_some()).expect("prompt block");
    assert_eq!(block_row.line, Line(2));
    assert_eq!(f.session.replies, vec!["\x1bP1$r0;1m\x1b\\"]);
}

#[test]
fn unicode_placeholder_places_and_clears_a_virtual_image() {
    let mut f = Fixture::new(20, 6);
    f.feed("\x1b_Ga=T,f=24,s=1,v=1,i=42,c=2,r=2,U=1,q=2;AAAA\x1b\\");
    assert!(f.pictures().is_empty(), "virtual placement waits for placeholder cells");
    f.feed("\x1b[2;3H\x1b[38;2;0;0;42m\u{10EEEE}\u{10EEEE}\x1b[3;3H\u{10EEEE}\u{10EEEE}\x1b[39m");
    let pics = f.pictures();
    assert_eq!(pics.len(), 1);
    assert_eq!((pics[0].image, pics[0].row, pics[0].column, pics[0].columns, pics[0].rows), (42, 1, 2, 2, 2));
    f.feed("\x1b[2;3H  \x1b[3;3H  ");
    assert!(f.pictures().is_empty());
}

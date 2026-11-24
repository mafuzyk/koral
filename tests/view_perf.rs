use std::time::Instant;

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use koral::blocks::BlockStore;
use koral::frame::{self, CursorView, Frame, Layout, draw_terminal};
use koral::marks::Mark;
use koral::render::Cell;
use koral::session::Session;
use koral::theme::Theme;
use koral::view::{Row, RowProjection, rows};

const PROMPT: &str = "\x1b]133;A\x1b\\$ \x1b]133;B\x1b\\";

struct Fixture {
    session: Session<VoidListener>,
    now: Instant,
}

impl Fixture {
    fn new(columns: usize, lines: usize) -> Self {
        let config = Config {
            scrolling_history: 128,
            ..Config::default()
        };
        let term = Term::new(config, &TermSize::new(columns, lines), VoidListener);
        Self {
            session: Session::new(term),
            now: Instant::now(),
        }
    }

    fn feed(&mut self, text: &str) {
        self.session.feed(text.as_bytes(), self.now);
    }

    fn command(&mut self, name: &str, output: &[&str]) {
        self.feed(PROMPT);
        self.feed(name);
        self.feed("\r\n\x1b]133;C\x1b\\");
        for line in output {
            self.feed(line);
            self.feed("\r\n");
        }
        self.feed("\x1b]133;D;0\x1b\\");
    }

    fn refresh<'a>(&self, projection: &'a mut RowProjection) -> &'a [Row] {
        projection.refresh(
            &self.session.term,
            &self.session.blocks,
            self.session.grid_revision(),
        );
        projection.rows()
    }
}

#[test]
fn projection_hits_on_unchanged_frames_and_invalidates_on_mutations() {
    let mut fixture = Fixture::new(40, 12);
    fixture.command("first", &["one", "two"]);
    fixture.command("second", &["three", "four"]);
    let mut projection = RowProjection::default();

    let initial = fixture.refresh(&mut projection).to_vec();
    assert_eq!(projection.hits(), 0, "the first projection is a miss");
    assert_eq!(fixture.refresh(&mut projection), initial);
    assert_eq!(projection.hits(), 1, "an unchanged frame is a hit");

    let id = initial
        .iter()
        .filter_map(|row| row.block)
        .find(|id| {
            fixture
                .session
                .blocks
                .get(*id)
                .is_some_and(|b| b.status.is_some())
        })
        .expect("a finished block");
    let revision = fixture.session.blocks.revision();
    fixture.session.blocks.toggle_fold(id);
    assert!(fixture.session.blocks.revision() > revision);

    let folded = fixture.refresh(&mut projection).to_vec();
    assert_eq!(projection.hits(), 1, "a fold toggle must invalidate");
    assert_ne!(folded, initial, "folding must change hidden counts");
    assert_eq!(fixture.refresh(&mut projection), folded);
    assert_eq!(projection.hits(), 2);

    fixture.command("third", &["five"]);
    let with_new_block = fixture.refresh(&mut projection).to_vec();
    assert_eq!(projection.hits(), 2, "a new block must invalidate");
    assert_ne!(with_new_block, folded);
    assert_eq!(fixture.refresh(&mut projection), with_new_block);
    assert_eq!(projection.hits(), 3);

    let before_scroll = projection.hits();
    fixture.session.term.scroll_display(Scroll::Delta(3));
    fixture.session.grid_changed();
    let scrolled = fixture.refresh(&mut projection).to_vec();
    assert_eq!(projection.hits(), before_scroll, "scroll must invalidate");
    assert_eq!(
        scrolled, with_new_block,
        "scroll changes a cache key, not rows"
    );
    assert_eq!(fixture.refresh(&mut projection), scrolled);
    assert_eq!(projection.hits(), before_scroll + 1);

    let before_resize = projection.hits();
    fixture.session.term.resize(TermSize::new(24, 12));
    fixture.session.grid_changed();
    let resized = fixture.refresh(&mut projection).to_vec();
    assert_eq!(projection.hits(), before_resize, "resize must invalidate");
    assert_eq!(fixture.refresh(&mut projection), resized);
    assert_eq!(projection.hits(), before_resize + 1);
}

#[test]
fn viewport_follows_scroll_without_invalidating_the_projection() {
    let mut fixture = Fixture::new(20, 10);
    for number in 0..24 {
        fixture.feed(&format!("line {number}\r\n"));
    }
    let mut projection = RowProjection::default();
    fixture.refresh(&mut projection);
    let hits = projection.hits();

    let top = frame::visible_top(projection.rows(), 6, 0);
    let expected: Vec<Row> =
        rows(&fixture.session.term, &fixture.session.blocks)[top..top + 6].to_vec();
    assert_eq!(projection.viewport(6, 0), expected);
    assert_eq!(projection.hits(), hits);

    let top = frame::visible_top(projection.rows(), 6, 4);
    let expected: Vec<Row> =
        rows(&fixture.session.term, &fixture.session.blocks)[top..top + 6].to_vec();
    assert_eq!(projection.viewport(6, 4), expected);
    assert_eq!(projection.hits(), hits, "scrolling reuses the cached rows");
    assert_ne!(projection.viewport(6, 0), projection.viewport(6, 4));
}

#[test]
fn revision_advances_for_every_row_relevant_block_change() {
    let mut store = BlockStore::default();
    let now = Instant::now();
    let start = store.revision();
    store.apply(Mark::Cwd("/tmp".to_string()), now);
    assert_eq!(store.revision(), start, "cwd alone does not change rows");
    store.apply(Mark::PromptStart, now);
    let after_new_block = store.revision();
    assert_eq!(after_new_block, start + 1, "a new block changes rows");
    let id = store.apply(Mark::PromptEnd, now).expect("an anchor");
    store.apply(Mark::CommandStart(Some("ls".to_string())), now);
    let before_status = store.revision();
    store.apply(Mark::CommandEnd(Some(0)), now);
    assert_eq!(store.revision(), before_status + 1);

    let before_fold = store.revision();
    store.toggle_fold(id);
    assert!(store.revision() > before_fold);
    let before_prompt = store.revision();
    store.set_prompt_lines(id, 3);
    assert!(store.revision() > before_prompt);
    assert_eq!(
        store.revision(),
        store.revision(),
        "revisions are monotonic"
    );
}

/// A tiny deterministic generator keeps the mutation sequence reproducible without a dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
}

#[test]
fn cached_rows_match_the_uncached_path_across_a_seeded_mutation_sequence() {
    let mut fixture = Fixture::new(32, 10);
    let mut projection = RowProjection::default();
    let mut rng = Rng(0x5eed_1234);
    let mut misses = 0;
    for step in 0..180 {
        let branch = rng.next() % 9;
        match branch {
            0 => fixture.feed(&format!("step {step}: some output\r\n")),
            1 => fixture.command(&format!("cmd{step}"), &["alpha", "beta"]),
            2 => {
                let candidates: Vec<u32> = rows(&fixture.session.term, &fixture.session.blocks)
                    .iter()
                    .filter_map(|row| row.block)
                    .collect();
                if let Some(id) = candidates.get((rng.next() as usize) % candidates.len().max(1)) {
                    fixture.session.blocks.toggle_fold(*id);
                }
            }
            3 => fixture.feed("\x1b[?1049h"),
            4 => fixture.feed("\x1b[?1049l"),
            5 => {
                let columns = if rng.next().is_multiple_of(2) { 28 } else { 36 };
                fixture.session.term.resize(TermSize::new(columns, 10));
                fixture.session.grid_changed();
            }
            6 => fixture.feed("\x1b]133;A\x1b\\\r\ninfo\r\n$ \x1b]133;B\x1b\\"),
            7 => fixture.session.term.scroll_display(Scroll::Delta(2)),
            _ => fixture.feed("\x1b]7;file:///tmp\x1b\\"),
        }

        let rebuilt = !projection.refresh(
            &fixture.session.term,
            &fixture.session.blocks,
            fixture.session.grid_revision(),
        );
        let cached = projection.rows().to_vec();
        let uncached = rows(&fixture.session.term, &fixture.session.blocks);
        assert_eq!(cached, uncached, "mutation step {step} desynchronized rows");
        if rebuilt {
            misses += 1;
        }
        // A second refresh with no intervening PTY damage must be a hit.
        let hits = projection.hits();
        fixture.refresh(&mut projection);
        assert_eq!(
            projection.hits(),
            hits + 1,
            "step {step} should have exactly one unchanged-frame hit"
        );

        for (lines, scroll) in [(4, 0), (6, 2), (3, 9)] {
            let top = frame::visible_top(&uncached, lines, scroll);
            let expected = &uncached[top..uncached.len().min(top + lines)];
            assert_eq!(
                projection.viewport(lines, scroll),
                expected,
                "viewport mismatch at step {step}"
            );
        }
    }

    assert!(
        misses > 20,
        "relevant mutations must rebuild rows, got {misses} misses"
    );
    assert!(
        projection.hits() > 20,
        "unchanged frames must reuse the cache"
    );
}

#[test]
fn frame_reset_keeps_lengths_zero_and_capacities_intact() {
    let mut fixture = Fixture::new(40, 16);
    for number in 0..20 {
        fixture.feed(&format!("row {number}\r\n"));
    }
    let layout = Layout {
        width: 640.0,
        height: 320.0,
        scale: 1.0,
        padding: 0.0,
        cell: Cell {
            width: 8.0,
            height: 16.0,
        },
    };
    let mut frame = Frame::default();
    let projected = rows(&fixture.session.term, &fixture.session.blocks);
    draw_terminal(
        &mut frame,
        &fixture.session.term,
        &fixture.session.blocks,
        &projected,
        0,
        CursorView {
            focused: true,
            visible: true,
        },
        layout,
        &Theme::default(),
    );

    assert!(!frame.quads.is_empty());
    assert!(!frame.lines.is_empty());
    let quads = frame.quads.capacity();
    let lines = frame.lines.capacity();
    assert!(quads > 0 && lines > 0);
    let lines_before = frame.lines.len();

    frame.reset();
    assert_eq!(frame.quads.len(), 0);
    assert_eq!(frame.lines.len(), 0);
    assert_eq!(frame.pictures.list.len(), 0);
    assert!(frame.quads.capacity() >= quads);
    assert!(frame.lines.capacity() >= lines);

    draw_terminal(
        &mut frame,
        &fixture.session.term,
        &fixture.session.blocks,
        &projected,
        0,
        CursorView {
            focused: true,
            visible: true,
        },
        layout,
        &Theme::default(),
    );
    assert_eq!(
        frame.lines.len(),
        lines_before,
        "a second frame draws the same rows"
    );
}

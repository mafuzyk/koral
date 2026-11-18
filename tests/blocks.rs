use std::time::{Duration, Instant};

use koral::blocks::{Activity, BlockStore, MAX_BLOCKS};
use koral::marks::Mark;

fn cmd(text: &str) -> Mark {
    Mark::CommandStart(Some(text.to_string()))
}

fn run(store: &mut BlockStore, now: Instant, text: &str, status: i32) {
    store.apply(Mark::PromptStart, now);
    store.apply(Mark::PromptEnd, now);
    store.apply(cmd(text), now);
    store.apply(Mark::CommandEnd(Some(status)), now + Duration::from_secs(2));
}

#[test]
fn a_full_cycle_records_the_command() {
    let mut store = BlockStore::default();
    let now = Instant::now();
    store.apply(Mark::Cwd("/tmp".to_string()), now);
    run(&mut store, now, "ls", 3);

    let block = store.latest().unwrap();
    assert_eq!(block.command.as_deref(), Some("ls"));
    assert_eq!(block.cwd.as_deref(), Some("/tmp"));
    assert_eq!(block.status, Some(3));
    assert_eq!(block.duration, Some(Duration::from_secs(2)));
}

#[test]
fn prompt_end_asks_for_an_anchor_every_time_with_the_same_id() {
    let mut store = BlockStore::default();
    let now = Instant::now();
    assert_eq!(store.apply(Mark::PromptStart, now), None);
    let first = store.apply(Mark::PromptEnd, now);
    assert!(first.is_some());
    assert_eq!(store.apply(Mark::PromptEnd, now), first);
}

#[test]
fn a_new_prompt_without_a_command_replaces_the_abandoned_block() {
    let mut store = BlockStore::default();
    let now = Instant::now();
    store.apply(Mark::PromptStart, now);
    let abandoned = store.apply(Mark::PromptEnd, now).unwrap();
    store.apply(Mark::PromptStart, now);
    let next = store.apply(Mark::PromptEnd, now).unwrap();

    assert_ne!(abandoned, next);
    assert_eq!(store.len(), 1);
    assert!(store.get(abandoned).is_none());
}

#[test]
fn command_end_at_the_prompt_records_a_parse_error() {
    let mut store = BlockStore::default();
    let now = Instant::now();
    store.apply(Mark::PromptStart, now);
    store.apply(Mark::PromptEnd, now);
    store.apply(Mark::CommandEnd(Some(2)), now);

    let block = store.latest().unwrap();
    assert_eq!(block.command, None);
    assert_eq!(block.status, Some(2));
    assert_eq!(store.activity(), Activity::Failed);
}

#[test]
fn prompt_marks_printed_by_a_running_command_are_ignored() {
    let mut store = BlockStore::default();
    let now = Instant::now();
    store.apply(Mark::PromptStart, now);
    store.apply(Mark::PromptEnd, now);
    store.apply(cmd("cat evil"), now);

    assert_eq!(store.apply(Mark::PromptStart, now), None);
    assert_eq!(store.apply(Mark::PromptEnd, now), None);
    store.apply(cmd("fake"), now);

    assert_eq!(store.len(), 1);
    assert_eq!(store.latest().unwrap().command.as_deref(), Some("cat evil"));
}

#[test]
fn a_cwd_printed_by_a_running_command_is_ignored() {
    let mut store = BlockStore::default();
    let now = Instant::now();
    store.apply(Mark::Cwd("/real".to_string()), now);
    store.apply(Mark::PromptStart, now);
    store.apply(Mark::PromptEnd, now);
    store.apply(cmd("cat evil"), now);
    store.apply(Mark::Cwd("/fake".to_string()), now);
    store.apply(Mark::CommandEnd(Some(0)), now);
    store.apply(Mark::PromptStart, now);

    assert_eq!(store.latest().unwrap().cwd.as_deref(), Some("/real"));
}

#[test]
fn command_marks_without_a_prompt_are_ignored() {
    let mut store = BlockStore::default();
    let now = Instant::now();
    store.apply(cmd("fake"), now);
    store.apply(Mark::CommandEnd(Some(1)), now);
    assert_eq!(store.apply(Mark::PromptEnd, now), None);
    assert_eq!(store.len(), 0);
}

#[test]
fn command_start_before_the_prompt_ends_is_ignored() {
    let mut store = BlockStore::default();
    let now = Instant::now();
    store.apply(Mark::PromptStart, now);
    store.apply(cmd("fake"), now);
    assert_eq!(store.activity(), Activity::Idle);
}

#[test]
fn nothing_starts_folded_and_toggling_is_the_only_way_in() {
    let mut store = BlockStore::default();
    let now = Instant::now();
    run(&mut store, now, "ls", 0);
    let id = store.latest().unwrap().id;

    assert!(!store.get(id).unwrap().folded);
    store.toggle_fold(id);
    assert!(store.get(id).unwrap().folded);
    store.toggle_fold(id);
    assert!(!store.get(id).unwrap().folded);
}

#[test]
fn activity_follows_the_latest_command() {
    let mut store = BlockStore::default();
    let now = Instant::now();
    assert_eq!(store.activity(), Activity::Idle);

    run(&mut store, now, "true", 0);
    assert_eq!(store.activity(), Activity::Succeeded);

    store.apply(Mark::PromptStart, now);
    assert_eq!(store.activity(), Activity::Succeeded);
    store.apply(Mark::PromptEnd, now);
    store.apply(cmd("sleep 9"), now);
    assert_eq!(store.activity(), Activity::Running);

    store.apply(Mark::CommandEnd(Some(1)), now);
    assert_eq!(store.activity(), Activity::Failed);
}

#[test]
fn the_oldest_blocks_are_dropped_at_the_limit() {
    let mut store = BlockStore::default();
    let now = Instant::now();
    run(&mut store, now, "first", 0);
    let first = store.latest().unwrap().id;
    for _ in 0..MAX_BLOCKS {
        run(&mut store, now, "again", 0);
    }

    assert_eq!(store.len(), MAX_BLOCKS);
    assert!(store.get(first).is_none());
    assert_eq!(store.latest().unwrap().command.as_deref(), Some("again"));
}

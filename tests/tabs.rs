use std::time::Instant;

use koral::blocks::BlockStore;
use koral::marks::Mark;
use koral::tabs::{Hit, Ring, Tabs, clean_title, hit, layout, ring, title};

fn tabs(count: usize) -> Tabs<usize> {
    let mut tabs = Tabs::new(0);
    for item in 1..count {
        tabs.push(item);
    }
    tabs
}

fn items(tabs: &Tabs<usize>) -> Vec<usize> {
    tabs.iter().copied().collect()
}

#[test]
fn a_new_tab_goes_last_and_becomes_active() {
    let tabs = tabs(3);
    assert_eq!(items(&tabs), vec![0, 1, 2]);
    assert_eq!(tabs.active_index(), 2);
}

#[test]
fn closing_the_active_tab_activates_its_right_neighbour() {
    let mut tabs = tabs(3);
    tabs.select(1);
    assert_eq!(tabs.close(1), Some(1));
    assert_eq!(tabs.iter().nth(tabs.active_index()), Some(&2));
}

#[test]
fn closing_the_last_position_activates_the_new_last() {
    let mut tabs = tabs(3);
    tabs.close(2);
    assert_eq!(tabs.active_index(), 1);
}

#[test]
fn closing_a_tab_before_the_active_one_keeps_the_active_tab() {
    let mut tabs = tabs(3);
    tabs.close(0);
    assert_eq!(tabs.iter().nth(tabs.active_index()), Some(&2));
}

#[test]
fn closing_a_tab_after_the_active_one_keeps_the_active_tab() {
    let mut tabs = tabs(3);
    tabs.select(0);
    tabs.close(2);
    assert_eq!(tabs.active_index(), 0);
}

#[test]
fn closing_the_only_tab_leaves_none() {
    let mut tabs = tabs(1);
    tabs.close(0);
    assert!(tabs.is_empty());
}

#[test]
fn closing_an_unknown_index_changes_nothing() {
    let mut tabs = tabs(2);
    assert_eq!(tabs.close(9), None);
    assert_eq!(tabs.len(), 2);
}

#[test]
fn selecting_past_the_end_changes_nothing() {
    let mut tabs = tabs(2);
    tabs.select(5);
    assert_eq!(tabs.active_index(), 1);
}

#[test]
fn next_and_previous_wrap_around() {
    let mut tabs = tabs(3);
    tabs.next();
    assert_eq!(tabs.active_index(), 0);
    tabs.previous();
    assert_eq!(tabs.active_index(), 2);
}

#[test]
fn moving_the_active_tab_keeps_it_active() {
    let mut tabs = tabs(4);
    tabs.select(0);
    tabs.move_tab(0, 2);
    assert_eq!(items(&tabs), vec![1, 2, 0, 3]);
    assert_eq!(tabs.active_index(), 2);
}

#[test]
fn moving_another_tab_across_the_active_one_keeps_the_active_tab() {
    let mut tabs = tabs(4);
    tabs.select(2);
    tabs.move_tab(3, 0);
    assert_eq!(items(&tabs), vec![3, 0, 1, 2]);
    assert_eq!(tabs.iter().nth(tabs.active_index()), Some(&2));
}

fn at_prompt(cwd: &str) -> BlockStore {
    let mut blocks = BlockStore::default();
    let now = Instant::now();
    blocks.apply(Mark::Cwd(cwd.to_string()), now);
    blocks.apply(Mark::PromptStart, now);
    blocks
}

#[test]
fn title_is_the_directory_name_at_a_prompt() {
    assert_eq!(title(&at_prompt("/home/mafu/koral"), None, Some("/home/mafu")), "koral");
}

#[test]
fn title_is_a_tilde_at_home() {
    assert_eq!(title(&at_prompt("/home/mafu"), None, Some("/home/mafu")), "~");
}

#[test]
fn title_is_the_first_line_of_the_running_command() {
    let mut blocks = at_prompt("/tmp");
    let now = Instant::now();
    blocks.apply(Mark::PromptEnd, now);
    blocks.apply(Mark::CommandStart(Some("cargo test\nls".to_string())), now);
    assert_eq!(title(&blocks, None, None), "cargo test");
}

#[test]
fn title_without_any_marks_is_the_product_name() {
    assert_eq!(title(&BlockStore::default(), None, None), "koral");
}

#[test]
fn tabs_share_the_width_up_to_a_maximum() {
    let bar = layout(2, 1000.0, 36.0);
    assert_eq!(bar.tabs.len(), 2);
    assert_eq!(bar.tabs[0].width, 220.0);
    assert_eq!(bar.tabs[1].x, bar.tabs[0].x + 220.0);
}

#[test]
fn tabs_shrink_to_leave_room_for_the_new_tab_button() {
    let bar = layout(10, 600.0, 36.0);
    assert!(bar.new_tab.x + bar.new_tab.width <= 600.0);
    assert!(bar.tabs[9].x + bar.tabs[9].width <= bar.new_tab.x);
}

#[test]
fn clicks_find_the_tab_under_them() {
    let bar = layout(2, 1000.0, 36.0);
    let second = bar.tabs[1];
    assert_eq!(hit(&bar, second.x + 10.0, 10.0), Hit::Tab(1));
}

#[test]
fn clicks_on_the_right_end_of_a_tab_close_it() {
    let bar = layout(2, 1000.0, 36.0);
    let first = bar.tabs[0];
    assert_eq!(hit(&bar, first.x + first.width - 14.0, 18.0), Hit::Close(0));
}

#[test]
fn clicks_on_the_plus_open_a_tab() {
    let bar = layout(2, 1000.0, 36.0);
    assert_eq!(hit(&bar, bar.new_tab.x + 4.0, 10.0), Hit::New);
}

#[test]
fn clicks_elsewhere_hit_nothing() {
    let bar = layout(1, 1000.0, 36.0);
    assert_eq!(hit(&bar, 900.0, 10.0), Hit::Empty);
    assert_eq!(hit(&bar, 10.0, 50.0), Hit::Empty);
}

fn running(command: &str) -> BlockStore {
    let mut blocks = at_prompt("/tmp");
    let now = Instant::now();
    blocks.apply(Mark::PromptEnd, now);
    blocks.apply(Mark::CommandStart(Some(command.to_string())), now);
    blocks
}

#[test]
fn a_running_program_names_the_tab() {
    assert_eq!(title(&running("nvim"), Some("notes.md - nvim"), None), "notes.md - nvim");
}

#[test]
fn the_prompt_ignores_a_stale_title() {
    assert_eq!(title(&at_prompt("/home/mafu/koral"), Some("notes.md - nvim"), None), "koral");
}

#[test]
fn a_shell_without_marks_uses_the_program_title() {
    assert_eq!(title(&BlockStore::default(), Some("bash"), None), "bash");
}

#[test]
fn no_marks_and_no_title_is_koral() {
    assert_eq!(title(&BlockStore::default(), None, None), "koral");
}

#[test]
fn clean_title_drops_control_characters() {
    assert_eq!(clean_title("a\x1b]0;b\x07c"), Some("a]0;bc".to_string()));
    assert_eq!(clean_title("\x07"), None);
}

#[test]
fn clean_title_cuts_a_long_title() {
    assert_eq!(clean_title(&"a".repeat(1000)).unwrap().chars().count(), 256);
}

#[test]
fn a_bell_in_the_tab_in_view_does_nothing() {
    assert_eq!(ring(true, true), Ring { mark: false, urgent: false });
}

#[test]
fn a_bell_in_another_tab_marks_it() {
    assert_eq!(ring(false, true), Ring { mark: true, urgent: false });
}

#[test]
fn a_bell_behind_another_window_asks_for_attention() {
    assert_eq!(ring(true, false), Ring { mark: false, urgent: true });
}

#[test]
fn a_bell_in_a_hidden_tab_does_both() {
    assert_eq!(ring(false, false), Ring { mark: true, urgent: true });
}

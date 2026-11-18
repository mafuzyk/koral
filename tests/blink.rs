use std::time::{Duration, Instant};

use koral::blink::Blink;

fn after(start: Instant, millis: u64) -> Instant {
    start + Duration::from_millis(millis)
}

#[test]
fn shows_then_hides_then_shows() {
    let start = Instant::now();
    let blink = Blink::new(start);
    assert!(blink.visible(start));
    assert!(!blink.visible(after(start, 530)));
    assert!(blink.visible(after(start, 1060)));
}

#[test]
fn the_next_change_is_on_the_half_period() {
    let start = Instant::now();
    assert_eq!(Blink::new(start).next(after(start, 600)), Some(after(start, 1060)));
}

#[test]
fn stops_visible_after_fifteen_seconds() {
    let start = Instant::now();
    let blink = Blink::new(start);
    let late = after(start, 15_100);
    assert!(blink.visible(late));
    assert_eq!(blink.next(late), None);
}

#[test]
fn reset_restarts_the_clock() {
    let start = Instant::now();
    let mut blink = Blink::new(start);
    blink.reset(after(start, 530));
    assert!(blink.visible(after(start, 530)));
    assert!(!blink.visible(after(start, 1060)));
}

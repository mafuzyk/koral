use std::time::{Duration, Instant};

use koral::pty::Pacer;

#[test]
fn the_first_read_paints_at_once() {
    assert!(Pacer::default().read(Instant::now()));
}

#[test]
fn a_read_right_after_a_paint_waits() {
    let (mut pacer, now) = (Pacer::default(), Instant::now());
    pacer.read(now);
    assert!(!pacer.read(now + Duration::from_millis(1)));
}

#[test]
fn a_long_burst_keeps_painting() {
    let (mut pacer, now) = (Pacer::default(), Instant::now());
    pacer.read(now);
    assert!(pacer.read(now + Duration::from_millis(20)));
}

#[test]
fn the_end_of_a_burst_paints_what_was_held_back() {
    let (mut pacer, now) = (Pacer::default(), Instant::now());
    pacer.read(now);
    pacer.read(now + Duration::from_millis(1));
    assert!(pacer.drained());
}

#[test]
fn the_end_of_a_burst_with_nothing_held_back_stays_quiet() {
    let (mut pacer, now) = (Pacer::default(), Instant::now());
    pacer.read(now);
    assert!(!pacer.drained());
}

use std::time::{Duration, Instant};

use koral::pty::{Pacer, refresh_interval};

#[test]
fn monitor_rate_controls_the_next_paint_during_a_burst() {
    for (rate, nanos) in [
        (60_000, 16_666_666),
        (120_000, 8_333_333),
        (144_000, 6_944_444),
        (240_000, 4_166_666),
    ] {
        let interval = refresh_interval(Some(rate));
        assert_eq!(interval, Duration::from_nanos(nanos));
        let mut pacer = Pacer::with_interval(interval);
        let now = Instant::now();
        assert!(pacer.read(now));
        assert!(!pacer.read(now + interval - Duration::from_nanos(1)));
        assert!(pacer.read(now + interval));
    }
    assert_eq!(refresh_interval(None), refresh_interval(Some(60_000)));
    assert_eq!(refresh_interval(Some(0)), refresh_interval(None));
}

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

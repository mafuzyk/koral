use std::time::{Duration, Instant};

use koral::notify::{Limiter, clean};

#[test]
fn the_limiter_allows_one_every_two_seconds() {
    let start = Instant::now();
    let mut limiter = Limiter::default();
    assert!(limiter.allow(start));
    assert!(!limiter.allow(start + Duration::from_millis(1999)));
    assert!(limiter.allow(start + Duration::from_secs(2)));
}

#[test]
fn clean_drops_control_characters() {
    assert_eq!(clean("a\x1b[31m\nb\x07", 100), "a[31mb");
}

#[test]
fn clean_cuts_at_the_limit_on_a_character_boundary() {
    assert_eq!(clean("açãob", 3), "açã");
}

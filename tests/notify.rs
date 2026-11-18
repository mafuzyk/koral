use std::time::{Duration, Instant};

use koral::notify::{Limiter, Notice, clean};

fn notice(title: &str) -> Notice {
    Notice { title: title.to_string(), body: String::new() }
}

#[test]
fn a_suppressed_notice_is_shown_when_the_spacing_passes() {
    let start = Instant::now();
    let mut limiter = Limiter::default();
    let second = notice("two");
    assert!(limiter.allow(start));
    assert!(!limiter.allow(start + Duration::from_millis(500)));
    limiter.hold(second.clone());
    assert_eq!(limiter.due(), Some(start + Duration::from_secs(2)));
    assert_eq!(limiter.release(start + Duration::from_millis(1500)), None);
    assert_eq!(limiter.release(start + Duration::from_secs(2)), Some(second));
    assert_eq!(limiter.due(), None);
    assert_eq!(limiter.release(start + Duration::from_secs(9)), None);
}

#[test]
fn the_newest_suppressed_notice_is_the_one_that_survives() {
    let start = Instant::now();
    let mut limiter = Limiter::default();
    assert!(limiter.allow(start));
    limiter.hold(notice("old"));
    limiter.hold(notice("new"));
    let shown = limiter.release(start + Duration::from_secs(2)).map(|notice| notice.title);
    assert_eq!(shown, Some("new".to_string()));
}

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

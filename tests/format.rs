use std::time::Duration;

use koral::format::{duration, lines};

#[test]
fn short_durations_keep_one_decimal() {
    assert_eq!(duration(Duration::from_millis(120)), "0.1s");
    assert_eq!(duration(Duration::from_millis(9_940)), "9.9s");
}

#[test]
fn longer_durations_are_whole_seconds() {
    assert_eq!(duration(Duration::from_secs(41)), "41s");
}

#[test]
fn minutes_and_hours_show_two_units() {
    assert_eq!(duration(Duration::from_secs(125)), "2m 5s");
    assert_eq!(duration(Duration::from_secs(3_780)), "1h 3m");
}

#[test]
fn a_tenth_that_rounds_up_crosses_to_whole_seconds() {
    assert_eq!(duration(Duration::from_millis(9_940)), "9.9s");
    assert_eq!(duration(Duration::from_millis(9_950)), "10s");
}

#[test]
fn a_second_that_rounds_up_crosses_to_a_minute() {
    assert_eq!(duration(Duration::from_millis(59_950)), "1m 0s");
}

#[test]
fn minutes_round_up_and_hours_keep_their_seconds_rounded() {
    assert_eq!(duration(Duration::from_secs(3_599)), "59m 59s");
    assert_eq!(duration(Duration::from_millis(3_599_600)), "1h 0m");
    assert_eq!(duration(Duration::from_secs(3_645)), "1h 1m");
}

#[test]
fn a_single_line_is_singular() {
    assert_eq!(lines(1), "1 line");
}

#[test]
fn line_counts_shorten_past_a_thousand() {
    assert_eq!(lines(999), "999 lines");
    assert_eq!(lines(2_300), "2.3k lines");
}

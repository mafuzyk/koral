use std::time::{Duration, Instant};

use koral::config::PermissionPolicy;
use koral::notify::{Decision, Limiter, Notice, Notifier, clean};

fn notice(title: &str) -> Notice {
    Notice {
        title: title.to_string(),
        body: String::new(),
    }
}

#[test]
fn clipboard_and_notification_policies_are_independent() {
    let mut notifier = Notifier::with_policies(PermissionPolicy::Allow, PermissionPolicy::Deny);
    assert_eq!(
        notifier.policies(),
        (PermissionPolicy::Allow, PermissionPolicy::Deny)
    );
    assert_eq!(
        notifier.clipboard_write("ok".to_string(), Instant::now()),
        Decision::Performed
    );
    assert_eq!(
        notifier.notify(notice("blocked"), Instant::now()),
        Decision::Denied
    );

    notifier.set_policies(PermissionPolicy::Deny, PermissionPolicy::Allow);
    assert_eq!(
        notifier.policies(),
        (PermissionPolicy::Deny, PermissionPolicy::Allow)
    );
    assert_eq!(
        notifier.clipboard_write("blocked".to_string(), Instant::now()),
        Decision::Denied
    );
    assert_eq!(
        notifier.notify(notice("ok"), Instant::now()),
        Decision::Performed
    );
}

#[test]
fn changing_policies_discards_stale_pending_and_ready_requests() {
    let start = Instant::now();
    let mut notifier = Notifier::with_policies(PermissionPolicy::Allow, PermissionPolicy::Allow);
    assert_eq!(
        notifier.clipboard_write("ready".to_string(), start),
        Decision::Performed
    );
    assert_eq!(notifier.notify(notice("shown"), start), Decision::Performed);
    assert_eq!(notifier.take_ready_notice(start), Some(notice("shown")));
    assert_eq!(notifier.notify(notice("stale held"), start), Decision::Held);

    notifier.set_policies(PermissionPolicy::Ask, PermissionPolicy::Deny);
    assert!(notifier.take_ready_text().is_none());
    assert_eq!(notifier.notification_due(), None);
    assert_eq!(
        notifier.clipboard_write("pending".to_string(), start),
        Decision::Confirm
    );
    assert_eq!(
        notifier.take_confirmation().as_deref(),
        Some("Allow clipboard write?")
    );

    notifier.set_policies(PermissionPolicy::Deny, PermissionPolicy::Deny);
    assert_eq!(notifier.confirm(start), None);
    assert!(notifier.take_confirmation().is_none());
}

#[test]
fn confirmed_notifications_keep_the_same_spacing_as_allowed_ones() {
    let start = Instant::now();
    let mut notifier = Notifier::new(PermissionPolicy::Ask);
    assert_eq!(notifier.notify(notice("one"), start), Decision::Confirm);
    assert_eq!(notifier.confirm(start), Some(Decision::Performed));
    assert!(notifier.notification_due().is_some());
    assert!(notifier.take_ready_notice(start).is_some());

    let second = start + Duration::from_secs(1);
    assert_eq!(notifier.notify(notice("two"), second), Decision::Confirm);
    assert_eq!(notifier.confirm(second), Some(Decision::Performed));
    let expected = start + Duration::from_secs(2);
    assert_eq!(notifier.notification_due(), Some(expected));
    assert!(
        notifier
            .take_ready_notice(expected - Duration::from_millis(1))
            .is_none()
    );
    assert_eq!(notifier.take_ready_notice(expected), Some(notice("two")));
}

#[test]
fn a_single_allowed_notice_is_shown_after_the_spacing_passes() {
    let start = Instant::now();
    let mut notifier = Notifier::new(PermissionPolicy::Allow);
    assert_eq!(notifier.notify(notice("one"), start), Decision::Performed);
    assert_eq!(notifier.notification_due(), Some(start));
    assert_eq!(
        notifier.take_ready_notice(start + Duration::from_millis(500)),
        Some(notice("one"))
    );
    assert_eq!(notifier.notification_due(), None);
}

#[test]
fn a_second_allowed_notice_waits_for_the_delivery_window() {
    let start = Instant::now();
    let mut notifier = Notifier::new(PermissionPolicy::Allow);
    assert_eq!(notifier.notify(notice("one"), start), Decision::Performed);
    assert_eq!(
        notifier.take_ready_notice(start + Duration::from_millis(500)),
        Some(notice("one"))
    );
    let expected = start + Duration::from_millis(500) + Duration::from_secs(2);
    assert_eq!(
        notifier.notify(notice("old"), start + Duration::from_secs(1)),
        Decision::Held
    );
    notifier.set_policies(PermissionPolicy::Deny, PermissionPolicy::Allow);
    assert_eq!(notifier.notification_due(), Some(expected));
    assert_eq!(
        notifier.notify(notice("two"), start + Duration::from_secs(2)),
        Decision::Held
    );
    assert_eq!(notifier.notification_due(), Some(expected));
    assert!(
        notifier
            .take_ready_notice(expected - Duration::from_millis(1))
            .is_none()
    );
    assert_eq!(notifier.take_ready_notice(expected), Some(notice("two")));
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
    assert_eq!(
        limiter.release(start + Duration::from_secs(2)),
        Some(second)
    );
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
    let shown = limiter
        .release(start + Duration::from_secs(2))
        .map(|notice| notice.title);
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
fn default_clipboard_ask_is_not_overridden_by_notification_allow() {
    let (config, errors) = koral::config::parse("");
    assert!(errors.is_empty());
    let now = Instant::now();
    let mut notifier = Notifier::with_policies(config.clipboard_write, config.notifications);
    assert_eq!(
        notifier.clipboard_write("text".into(), now),
        Decision::Confirm
    );
    notifier.set_policies(config.clipboard_write, PermissionPolicy::Deny);
    assert!(notifier.take_confirmation().is_some());
    assert!(notifier.take_ready_text().is_none());
}

#[test]
fn clean_drops_control_characters() {
    assert_eq!(clean("a\x1b[31m\nb\x07", 100), "a[31mb");
}

#[test]
fn clean_cuts_at_the_limit_on_a_character_boundary() {
    assert_eq!(clean("açãob", 3), "açã");
}

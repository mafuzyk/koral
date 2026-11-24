use std::time::{Duration, Instant};

use koral::config::{Config, PermissionPolicy, parse};
use koral::graphics::Graphics;
use koral::marks::{MAX_MARK, MAX_STATUS, Mark, Piece, Scanner};
use koral::notify::{Decision, Notice, Notifier, WriteOrigin, clipboard_write_decision};

fn start() -> Instant {
    Instant::now()
}

fn notice() -> Notice {
    Notice {
        title: "Build".to_string(),
        body: "done".to_string(),
    }
}

fn scan(chunks: &[&[u8]]) -> Vec<Vec<u8>> {
    let mut scanner = Scanner::default();
    let mut out: Vec<Vec<u8>> = Vec::new();
    for chunk in chunks {
        scanner.feed(chunk, &mut |piece| {
            if let Piece::Bytes(bytes) = piece {
                match out.last_mut() {
                    Some(last) if !last.is_empty() && last[0] != 0 => last.extend_from_slice(bytes),
                    _ => out.push(bytes.to_vec()),
                }
            }
        });
    }
    out
}

#[test]
fn default_permissions_stay_private() {
    let config = Config::default();
    assert_eq!(config.clipboard_write, PermissionPolicy::Ask);
    assert_eq!(config.notifications, PermissionPolicy::Allow);
    assert!(!config.clipboard_read);
}

#[test]
fn user_clipboard_writes_bypass_every_permission_policy() {
    for policy in [
        PermissionPolicy::Allow,
        PermissionPolicy::Ask,
        PermissionPolicy::Deny,
    ] {
        let mut notifier = Notifier::new(policy);
        assert_eq!(
            clipboard_write_decision(WriteOrigin::User, policy),
            Decision::Performed
        );
        assert_eq!(
            notifier.clipboard_write_from(WriteOrigin::User, "user text".to_string(), start()),
            Decision::Performed
        );
        assert_eq!(notifier.take_ready_text().as_deref(), Some("user text"));
        assert!(notifier.take_confirmation().is_none());
    }
}

#[test]
fn program_clipboard_writes_follow_the_permission_policy() {
    assert_eq!(
        clipboard_write_decision(WriteOrigin::Program, PermissionPolicy::Allow),
        Decision::Performed
    );
    assert_eq!(
        clipboard_write_decision(WriteOrigin::Program, PermissionPolicy::Ask),
        Decision::Confirm
    );
    assert_eq!(
        clipboard_write_decision(WriteOrigin::Program, PermissionPolicy::Deny),
        Decision::Denied
    );
}

#[test]
fn an_ask_program_copy_waits_without_touching_the_clipboard() {
    let mut notifier = Notifier::new(PermissionPolicy::Ask);
    assert_eq!(
        clipboard_write_decision(WriteOrigin::Program, notifier.policy()),
        Decision::Confirm
    );
    assert_eq!(
        notifier.clipboard_write_from(WriteOrigin::Program, "secret".to_string(), start()),
        Decision::Confirm
    );
    assert!(notifier.take_ready_text().is_none());
    assert_eq!(
        notifier.take_confirmation().as_deref(),
        Some("Allow clipboard write?")
    );
}

#[test]
fn a_denied_policy_refuses_program_copies_but_not_user_copies() {
    let mut notifier = Notifier::new(PermissionPolicy::Deny);
    assert_eq!(
        clipboard_write_decision(WriteOrigin::User, notifier.policy()),
        Decision::Performed
    );
    assert_eq!(
        clipboard_write_decision(WriteOrigin::Program, notifier.policy()),
        Decision::Denied
    );
    assert_eq!(
        notifier.clipboard_write_from(WriteOrigin::Program, "secret".to_string(), start()),
        Decision::Denied
    );
    assert_eq!(
        notifier.clipboard_write_from(WriteOrigin::User, "user text".to_string(), start()),
        Decision::Performed
    );
    assert_eq!(notifier.take_ready_text().as_deref(), Some("user text"));
    assert!(notifier.take_ready_text().is_none());
    assert!(notifier.take_confirmation().is_none());
}

#[test]
fn an_allowed_program_copy_is_performed_immediately() {
    let mut notifier = Notifier::new(PermissionPolicy::Allow);
    assert_eq!(
        clipboard_write_decision(WriteOrigin::Program, notifier.policy()),
        Decision::Performed
    );
    assert_eq!(
        notifier.clipboard_write_from(WriteOrigin::Program, "public".to_string(), start()),
        Decision::Performed
    );
    assert_eq!(notifier.take_ready_text().as_deref(), Some("public"));
    assert!(notifier.take_confirmation().is_none());
}

#[test]
fn policy_settings_parse_and_reject_other_words() {
    let (config, errors) = parse("clipboard_write deny\nnotifications ask\n");
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(config.clipboard_write, PermissionPolicy::Deny);
    assert_eq!(config.notifications, PermissionPolicy::Ask);
    assert!(!config.clipboard_read);

    let errors = parse("clipboard_write maybe\nnotifications 1\n").1;
    assert_eq!(errors.len(), 2);
}

#[test]
fn a_denied_clipboard_store_is_never_written() {
    let mut notifier = Notifier::new(PermissionPolicy::Deny);
    assert_eq!(
        notifier.clipboard_write("secret".to_string(), start()),
        Decision::Denied
    );
    assert!(notifier.take_confirmation().is_none());
}

#[test]
fn a_denied_notification_is_never_shown() {
    let mut notifier = Notifier::new(PermissionPolicy::Deny);
    assert_eq!(notifier.notify(notice(), start()), Decision::Denied);
    assert!(notifier.take_ready_notice(start()).is_none());
}

#[test]
fn ask_holds_a_clipboard_write_for_confirmation_without_mutating_it() {
    let mut notifier = Notifier::new(PermissionPolicy::Ask);
    assert_eq!(
        notifier.clipboard_write("secret".to_string(), start()),
        Decision::Confirm
    );
    assert!(notifier.take_ready_text().is_none());
    assert_eq!(
        notifier.take_confirmation().as_deref(),
        Some("Allow clipboard write?")
    );
    assert_eq!(notifier.confirm(start()), Some(Decision::Performed));
    assert_eq!(notifier.take_ready_text().as_deref(), Some("secret"));
    assert!(notifier.take_confirmation().is_none());
}

#[test]
fn a_rejected_confirmation_discards_the_clipboard_write() {
    let mut notifier = Notifier::new(PermissionPolicy::Ask);
    assert_eq!(
        notifier.clipboard_write("secret".to_string(), start()),
        Decision::Confirm
    );
    assert_eq!(notifier.cancel(), Some(Decision::Denied));
    assert!(notifier.take_ready_text().is_none());
    assert!(notifier.take_confirmation().is_none());
}

#[test]
fn ask_serializes_clipboard_requests_and_alerts() {
    let mut notifier = Notifier::new(PermissionPolicy::Ask);
    assert_eq!(
        notifier.clipboard_write("first".to_string(), start()),
        Decision::Confirm
    );
    assert_eq!(
        notifier.clipboard_write("second".to_string(), start()),
        Decision::Denied
    );
    assert_eq!(notifier.notify(notice(), start()), Decision::Denied);

    assert_eq!(notifier.confirm(start()), Some(Decision::Performed));
    assert_eq!(notifier.take_ready_text().as_deref(), Some("first"));
    assert!(notifier.take_confirmation().is_none());
    assert_eq!(
        notifier.notify(notice(), start() + Duration::from_secs(3)),
        Decision::Confirm
    );
    assert_eq!(
        notifier.take_confirmation().as_deref(),
        Some("Allow notification?")
    );
}

#[test]
fn an_ask_notification_waits_for_confirmation() {
    let mut notifier = Notifier::new(PermissionPolicy::Ask);
    assert_eq!(notifier.notify(notice(), start()), Decision::Confirm);
    assert!(notifier.take_ready_notice(start()).is_none());
    assert_eq!(notifier.confirm(start()), Some(Decision::Performed));
    assert_eq!(notifier.take_ready_notice(start()), Some(notice()));
    assert!(notifier.take_confirmation().is_none());
}

#[test]
fn an_expired_confirmation_denies_the_pending_request() {
    let mut notifier = Notifier::new(PermissionPolicy::Ask);
    assert_eq!(
        notifier.clipboard_write("secret".to_string(), start()),
        Decision::Confirm
    );
    assert_eq!(
        notifier.confirm(start() + Duration::from_secs(4)),
        Some(Decision::Denied)
    );
    assert!(notifier.take_ready_text().is_none());
    assert!(notifier.take_confirmation().is_none());
}

#[test]
fn allowed_clipboard_and_notifications_respect_their_limits() {
    let mut notifier = Notifier::new(PermissionPolicy::Allow);
    assert_eq!(
        notifier.clipboard_write("one".to_string(), start()),
        Decision::Performed
    );
    assert_eq!(
        notifier.clipboard_write("two".to_string(), start()),
        Decision::Performed
    );
    assert_eq!(notifier.take_ready_text().as_deref(), Some("one"));
    assert_eq!(notifier.take_ready_text().as_deref(), Some("two"));
    assert!(notifier.take_ready_text().is_none());

    let now = start();
    assert_eq!(notifier.notify(notice(), now), Decision::Performed);
    assert_eq!(notifier.take_ready_notice(now), Some(notice()));
    assert_eq!(notifier.notify(notice(), now), Decision::Held);
    assert!(notifier.take_ready_notice(now).is_none());
    assert_eq!(
        notifier.take_ready_notice(now + Duration::from_secs(2)),
        Some(notice())
    );
}

#[test]
fn oversized_osc_aborts_and_normal_vt_resumes_in_one_feed() {
    let mut oversized = b"\x1b]777;notify;title;".to_vec();
    oversized.resize(MAX_MARK + 64, b'x');
    let mut input = oversized.clone();
    input.extend_from_slice(b"normal\x1b[31mred");

    let mut scanner = Scanner::default();
    let mut marks = 0;
    let mut text = Vec::new();
    scanner.feed(&input, &mut |piece| match piece {
        Piece::Bytes(bytes) => text.extend_from_slice(bytes),
        Piece::Mark(_) => marks += 1,
        Piece::Graphics(_) | Piece::Status(_) => {}
    });

    assert_eq!(marks, 0);
    let expected = [oversized.as_slice(), b"normal\x1b[31mred".as_slice()].concat();
    assert_eq!(text, expected);
}

#[test]
fn oversized_osc_aborts_and_resumes_across_chunk_boundaries() {
    let mut oversized = b"\x1b]133;C;cmdline_url=".to_vec();
    oversized.resize(MAX_MARK + 64, b'x');
    let mut scanner = Scanner::default();
    let mut marks = 0;
    let mut text = Vec::new();
    let mut sink = |piece: Piece<'_>| match piece {
        Piece::Bytes(bytes) => text.extend_from_slice(bytes),
        Piece::Mark(_) => marks += 1,
        Piece::Graphics(_) | Piece::Status(_) => {}
    };
    for part in oversized.chunks(4096) {
        scanner.feed(part, &mut sink);
    }
    scanner.feed(b"after\x1b]133;A\x07", &mut sink);

    assert_eq!(marks, 1);
    assert_eq!(&text[..oversized.len()], oversized.as_slice());
    assert_eq!(&text[oversized.len()..], b"after");
}

#[test]
fn an_oversized_status_aborts_and_resumes_normal_vt() {
    let mut oversized = b"\x1bP$q".to_vec();
    oversized.resize(MAX_STATUS + 64, b'x');
    oversized.extend_from_slice(b"\x1b\\");
    let mut input = oversized.clone();
    input.extend_from_slice(b"ok");

    let mut scanner = Scanner::default();
    let mut statuses = 0;
    let mut text = Vec::new();
    scanner.feed(&input, &mut |piece| match piece {
        Piece::Bytes(bytes) => text.extend_from_slice(bytes),
        Piece::Status(_) => statuses += 1,
        Piece::Mark(_) | Piece::Graphics(_) => {}
    });
    assert_eq!(statuses, 0);
    assert_eq!(text, [oversized.as_slice(), b"ok".as_slice()].concat());
}

#[test]
fn reset_abandons_a_partial_mark_without_dropping_its_bytes() {
    let mut scanner = Scanner::default();
    let mut marks = 0;
    let mut text = Vec::new();
    let mut sink = |piece: Piece<'_>| match piece {
        Piece::Bytes(bytes) => text.extend_from_slice(bytes),
        Piece::Mark(mark) => {
            assert_eq!(mark, Mark::PromptStart);
            marks += 1;
        }
        Piece::Graphics(_) | Piece::Status(_) => {}
    };
    for part in [b"\x1b]133;A\x1b" as &[u8], b"\x1bc", b"\x1b]133;A\x07"] {
        scanner.feed(part, &mut sink);
    }
    assert_eq!(marks, 1, "the mark before the reset was retained");
    assert_eq!(text, b"\x1b]133;A\x1b\x1bc");
}

#[test]
fn an_oversized_graphics_payload_releases_it_and_resumes() {
    let mut scanner = Scanner::default();
    let mut released = 0;
    let mut text = Vec::new();
    let mut input = b"\x1b_Ga=t;".to_vec();
    input.resize(4 * 1024 * 1024, b'A');
    input.extend_from_slice(b"tail");
    scanner.feed(&input, &mut |piece| {
        if let Piece::Bytes(bytes) = piece {
            released += bytes.len();
            text.extend_from_slice(bytes);
        }
    });
    assert!(released > 1024);
    assert!(text.ends_with(b"tail"));
    assert!(scan(&[b"tail"]).concat() == b"tail");
}

#[test]
fn a_burst_of_marks_cannot_grow_the_graphics_store_without_bound() {
    let mut graphics = Graphics::default();
    for id in 1..=4 {
        let payload = vec![0; 4 * 1024 * 1024];
        graphics.handle(format!("a=t,f=32,s=1024,v=1024,i={id},q=2").as_bytes());
        graphics.handle(&[b"G", format!("i={id}").as_bytes(), b";", payload.as_slice()].concat());
    }
    assert!(graphics.used() <= 5 * koral::graphics::MAX_IMAGE);
}

fn corrupt_inputs() -> Vec<u8> {
    let mut inputs = Vec::new();
    for byte in 0u8..=u8::MAX {
        if !matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b';' | b'=' | b' ' | b'\n') {
            continue;
        }
        inputs.extend_from_slice(b"\x1b]133;");
        inputs.push(byte);
        inputs.extend_from_slice(b"\x1b\\");
        inputs.extend_from_slice(b"\x1b_G");
        inputs.push(byte);
        inputs.extend_from_slice(b"\x1b\\");
        inputs.extend_from_slice(b"\x1bP$q");
        inputs.push(byte);
        inputs.extend_from_slice(b"\x1b\\");
        inputs.extend_from_slice(b"\x1bc");
        inputs.push(byte);
    }
    inputs
}

#[test]
fn corrupted_input_never_panics_any_parser() {
    let mut scanner = Scanner::default();
    let input = corrupt_inputs();
    scanner.feed(&input, &mut |_| {});
    let text = String::from_utf8_lossy(&input);
    let _ = koral::ipc::decode(&format!("send-text\t{text}"));
    let _ = parse(&format!("font {text}\n"));
}

#[test]
fn deterministic_seed_corpus_never_panics() {
    let mut scanner = Scanner::default();
    let mut graphics = Graphics::default();
    for seed in security_seeds() {
        for chunk in seed.chunks(7) {
            scanner.feed(chunk, &mut |piece| {
                let _ = match piece {
                    Piece::Bytes(bytes) => bytes.len(),
                    Piece::Graphics(bytes) => bytes.len(),
                    Piece::Status(bytes) => bytes.len(),
                    Piece::Mark(mark) => format!("{mark:?}").len(),
                };
            });
        }
        let _ = graphics.handle(&seed);
        let _ = koral::ipc::decode(&String::from_utf8_lossy(&seed));
        let _ = parse(&format!("font {}\n", String::from_utf8_lossy(&seed)));
    }
}

fn security_seeds() -> Vec<Vec<u8>> {
    [
        b"\x1b]133;A\x07".as_slice(),
        b"\x1b]133;C;cmdline_url=echo%20hi\x1b\\",
        b"\x1b]777;notify;title;body\x07",
        b"\x1b]7;file://host/home/user\x07",
        b"\x1bP$q\x1b\\",
        b"\x1b_Ga=t,f=24,s=1,v=1,i=1;AQID\x1b\\",
        b"\x1bc",
        b"\x1b]133;D;130\x07",
        b"plain text\x1b[31mred",
        b"\xff\xfe\x00",
    ]
    .into_iter()
    .map(|seed| seed.to_vec())
    .collect()
}

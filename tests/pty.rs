use koral::pty::device_reply;

#[test]
fn a_device_query_answers_as_a_vt220() {
    assert_eq!(device_reply("\x1b[?6c".to_string()), "\x1b[?62;c");
}

#[test]
fn other_replies_pass_through() {
    assert_eq!(device_reply("\x1b[0n".to_string()), "\x1b[0n");
}

use koral::marks::{MAX_GRAPHICS, MAX_MARK, MAX_STATUS, Mark, Piece, Scanner};

#[derive(Debug, PartialEq, Eq)]
enum Out {
    Bytes(Vec<u8>),
    Mark(Mark),
    Status(Vec<u8>),
}

fn scan(chunks: &[&[u8]]) -> Vec<Out> {
    let mut scanner = Scanner::default();
    let mut out: Vec<Out> = Vec::new();
    for chunk in chunks {
        scanner.feed(chunk, &mut |piece| match piece {
            Piece::Bytes(bytes) => match out.last_mut() {
                Some(Out::Bytes(last)) => last.extend_from_slice(bytes),
                _ => out.push(Out::Bytes(bytes.to_vec())),
            },
            Piece::Mark(mark) => out.push(Out::Mark(mark)),
            Piece::Graphics(_) => {}
            Piece::Status(query) => out.push(Out::Status(query.to_vec())),
        });
    }
    out
}

fn bytes(text: &[u8]) -> Out {
    Out::Bytes(text.to_vec())
}

#[test]
fn escape_at_mark_cap_starts_the_next_mark_across_chunks() {
    let mut oversized = b"\x1b]133;".to_vec();
    oversized.resize(MAX_MARK - 1, b'x');
    let mut stream = oversized.clone();
    stream.extend_from_slice(b"\x1b]133;A\x07");
    for split in [0, MAX_MARK - 1, MAX_MARK, MAX_MARK + 1, stream.len()] {
        assert_eq!(
            scan(&[&stream[..split], &stream[split..]]),
            vec![bytes(&oversized), Out::Mark(Mark::PromptStart)]
        );
    }
}

#[test]
fn repeated_escapes_before_reset_are_not_lost() {
    assert_eq!(
        scan(&[b"\x1b]13\x1b", b"\x1b", b"\x1bcplain"]),
        vec![bytes(b"\x1b]13\x1b\x1b\x1bcplain")]
    );
}

#[test]
fn plain_output_passes_through() {
    assert_eq!(scan(&[b"hello\r\n"]), vec![bytes(b"hello\r\n")]);
}

#[test]
fn prompt_marks_are_taken_out_of_the_stream() {
    assert_eq!(
        scan(&[b"a\x1b]133;A\x1b\\$ \x1b]133;B\x1b\\b"]),
        vec![
            bytes(b"a"),
            Out::Mark(Mark::PromptStart),
            bytes(b"$ "),
            Out::Mark(Mark::PromptEnd),
            bytes(b"b"),
        ]
    );
}

#[test]
fn bel_terminates_a_mark() {
    assert_eq!(
        scan(&[b"\x1b]133;A\x07"]),
        vec![Out::Mark(Mark::PromptStart)]
    );
}

#[test]
fn command_start_decodes_the_command_line() {
    assert_eq!(
        scan(&[b"\x1b]133;C;cmdline_url=echo%20%22ol%C3%A1%22%0Als\x1b\\"]),
        vec![Out::Mark(Mark::CommandStart(Some(
            "echo \"olá\"\nls".to_string()
        )))]
    );
}

#[test]
fn command_start_without_a_command_line_is_valid() {
    assert_eq!(
        scan(&[b"\x1b]133;C\x1b\\"]),
        vec![Out::Mark(Mark::CommandStart(None))]
    );
}

#[test]
fn bad_percent_escapes_lose_the_command_line_but_not_the_mark() {
    assert_eq!(
        scan(&[b"\x1b]133;C;cmdline_url=%zz\x1b\\"]),
        vec![Out::Mark(Mark::CommandStart(None))]
    );
}

#[test]
fn command_end_carries_the_status() {
    assert_eq!(
        scan(&[b"\x1b]133;D;130\x1b\\"]),
        vec![Out::Mark(Mark::CommandEnd(Some(130)))]
    );
}

#[test]
fn command_end_without_a_status_is_valid() {
    assert_eq!(
        scan(&[b"\x1b]133;D\x1b\\"]),
        vec![Out::Mark(Mark::CommandEnd(None))]
    );
}

#[test]
fn cwd_mark_decodes_the_path_and_drops_the_host() {
    assert_eq!(
        scan(&[b"\x1b]7;file://cachyos/home/mafu/meu%20site\x1b\\"]),
        vec![Out::Mark(Mark::Cwd("/home/mafu/meu site".to_string()))]
    );
}

#[test]
fn cwd_mark_with_an_empty_host_is_valid() {
    assert_eq!(
        scan(&[b"\x1b]7;file:///tmp\x1b\\"]),
        vec![Out::Mark(Mark::Cwd("/tmp".to_string()))]
    );
}

#[test]
fn other_osc_sequences_are_left_for_the_terminal() {
    let title = b"\x1b]0;title\x07";
    assert_eq!(scan(&[title]), vec![bytes(title)]);
    let similar = b"\x1b]1337;x\x07";
    assert_eq!(scan(&[similar]), vec![bytes(similar)]);
}

#[test]
fn other_escape_sequences_are_left_for_the_terminal() {
    let colour = b"\x1b[31mred\x1b[0m";
    assert_eq!(scan(&[colour]), vec![bytes(colour)]);
}

#[test]
fn unknown_133_kinds_are_left_for_the_terminal() {
    let unknown = b"\x1b]133;Z\x1b\\";
    assert_eq!(scan(&[unknown]), vec![bytes(unknown)]);
}

#[test]
fn a_mark_split_at_every_byte_still_parses() {
    let input = b"x\x1b]133;D;7\x1b\\y";
    let chunks: Vec<&[u8]> = input.chunks(1).collect();
    assert_eq!(
        scan(&chunks),
        vec![
            bytes(b"x"),
            Out::Mark(Mark::CommandEnd(Some(7))),
            bytes(b"y")
        ]
    );
}

#[test]
fn a_lone_escape_at_a_chunk_end_is_held_until_it_can_be_decided() {
    assert_eq!(scan(&[b"a\x1b", b"[0m"]), vec![bytes(b"a\x1b[0m")]);
}

#[test]
fn a_long_plain_run_finds_the_next_escape_without_crossing_the_chunk() {
    let plain = vec![b'.'; 70_000];
    let mut input = b"prefix".to_vec();
    input.extend_from_slice(&plain);
    input.extend_from_slice(b"\x1b]133;A\x1b\\tail");
    let out = scan(&[&input]);
    assert_eq!(
        out,
        vec![
            bytes(&input[..input.len() - b"\x1b]133;A\x1b\\tail".len()]),
            Out::Mark(Mark::PromptStart),
            bytes(b"tail"),
        ]
    );
}

#[test]
fn an_escape_landing_on_the_last_byte_of_a_chunk_is_held() {
    let mark = b"\x1b]133;D;7\x1b\\";
    let (head, tail) = mark.split_at(mark.len() - 1);
    assert_eq!(
        scan(&[b"x", head, tail, b"y"]),
        vec![
            bytes(b"x"),
            Out::Mark(Mark::CommandEnd(Some(7))),
            bytes(b"y")
        ]
    );
}

#[test]
fn consecutive_escapes_at_chunk_edges_keep_every_byte() {
    assert_eq!(
        scan(&[b"a\x1b\x1b", b"\x1bc", b"[0m"]),
        vec![bytes(b"a\x1b\x1b\x1bc[0m")]
    );
    assert_eq!(
        scan(&[b"\x1b\x1b]133;A\x1b\\"]),
        vec![bytes(b"\x1b"), Out::Mark(Mark::PromptStart)]
    );
}

#[test]
fn an_unterminated_mark_is_released_at_the_size_limit() {
    let mut input = b"\x1b]133;C;cmdline_url=".to_vec();
    input.resize(70_000, b'a');
    let out = scan(&[&input]);
    assert_eq!(out, vec![bytes(&input)]);
}

#[test]
fn an_escape_inside_a_mark_abandons_it_and_keeps_every_byte() {
    let input = b"\x1b]133;D\x1b[0m";
    assert_eq!(scan(&[input]), vec![bytes(input)]);
}

#[test]
fn a_graphics_command_leaves_the_stream_whole() {
    let mut scanner = Scanner::default();
    let mut seen = Vec::new();
    scanner.feed(b"a\x1b_Ga=t;QUJD\x1b\\b", &mut |piece| match piece {
        Piece::Bytes(bytes) => seen.push(format!("bytes {}", String::from_utf8_lossy(bytes))),
        Piece::Graphics(body) => seen.push(format!("graphics {}", String::from_utf8_lossy(body))),
        Piece::Mark(_) | Piece::Status(_) => {}
    });
    assert_eq!(seen, ["bytes a", "graphics Ga=t;QUJD", "bytes b"]);
}

#[test]
fn a_graphics_command_split_across_reads_is_still_found() {
    let mut scanner = Scanner::default();
    let mut found = 0;
    let mut sink = |piece: Piece<'_>| found += i32::from(matches!(piece, Piece::Graphics(_)));
    for part in [&b"\x1b"[..], b"_", b"Ga=t;", b"QUJD\x1b", b"\\"] {
        scanner.feed(part, &mut sink);
    }
    assert_eq!(found, 1);
}

#[test]
fn an_endless_graphics_command_is_let_go() {
    let mut scanner = Scanner::default();
    let mut found = false;
    let endless = [b"\x1b_Ga=t;".to_vec(), vec![b'A'; 4 * 1024 * 1024]].concat();
    scanner.feed(&endless, &mut |piece| {
        found |= matches!(piece, Piece::Bytes(bytes) if bytes.len() > 1024)
    });
    assert!(found);
}

#[test]
fn other_application_commands_pass_through() {
    let mut scanner = Scanner::default();
    let mut text = Vec::new();
    scanner.feed(b"\x1b_Xhello\x1b\\", &mut |piece| {
        if let Piece::Bytes(bytes) = piece {
            text.extend_from_slice(bytes);
        }
    });
    assert_eq!(text, b"\x1b_Xhello\x1b\\");
}

fn notify(title: &str, body: &str) -> Out {
    Out::Mark(Mark::Notify {
        title: title.to_string(),
        body: body.to_string(),
    })
}

#[test]
fn osc_777_is_a_notification() {
    assert_eq!(
        scan(&[b"\x1b]777;notify;Build;done; ok\x1b\\"]),
        vec![notify("Build", "done; ok")]
    );
}

#[test]
fn osc_9_is_a_notification() {
    assert_eq!(scan(&[b"\x1b]9;done\x07"]), vec![notify("", "done")]);
}

#[test]
fn osc9_progress_is_not_a_notification() {
    assert_eq!(
        scan(&[b"\x1b]9;4;1;50\x07"]),
        vec![Out::Mark(Mark::Progress(koral::marks::Progress::Normal(
            50
        )))]
    );
    assert_eq!(
        scan(&[b"\x1b]9;4;0\x07\x1b]9;4;2;120\x07\x1b]9;4;3\x07\x1b]9;4;4;75\x07"]),
        vec![
            Out::Mark(Mark::Progress(koral::marks::Progress::Clear)),
            Out::Mark(Mark::Progress(koral::marks::Progress::Error(100))),
            Out::Mark(Mark::Progress(koral::marks::Progress::Indeterminate)),
            Out::Mark(Mark::Progress(koral::marks::Progress::Paused(75))),
        ]
    );
    // Non-progress numeric `9;<N>;...` or malformed `9;4;99` pass through as raw bytes and never notify.
    assert_eq!(
        scan(&[b"\x1b]9;4;99;10\x07"]),
        vec![bytes(b"\x1b]9;4;99;10\x07")]
    );
    assert_eq!(
        scan(&[b"\x1b]9;2;foo\x07"]),
        vec![bytes(b"\x1b]9;2;foo\x07")]
    );
}

#[test]
fn osc22_cursor_shape_parses_and_resets() {
    assert_eq!(
        scan(&[b"\x1b]22;pointer\x07"]),
        vec![Out::Mark(Mark::CursorShape(Some("pointer".to_string())))]
    );
    assert_eq!(
        scan(&[b"\x1b]22;default\x1b\\"]),
        vec![Out::Mark(Mark::CursorShape(None))]
    );
    assert_eq!(
        scan(&[b"\x1b]22;\x07"]),
        vec![Out::Mark(Mark::CursorShape(None))]
    );
}

#[test]
fn a_notification_split_across_reads_is_still_found() {
    assert_eq!(
        scan(&[b"\x1b]77", b"7;noti", b"fy;a;b\x07"]),
        vec![notify("a", "b")]
    );
}

#[test]
fn an_unknown_777_stays_in_the_stream() {
    let text = b"\x1b]777;foo;a\x07";
    assert_eq!(
        scan(&[text])
            .into_iter()
            .flat_map(|out| match out {
                Out::Bytes(part) => part,
                Out::Mark(_) | Out::Status(_) => panic!("not a mark"),
            })
            .collect::<Vec<u8>>(),
        text
    );
}

#[test]
fn a_status_query_split_across_reads_is_taken_out() {
    assert_eq!(
        scan(&[b"a\x1bP$", b"qm\x1b", b"\\b"]),
        vec![bytes(b"a"), Out::Status(b"m".to_vec()), bytes(b"b")]
    );
}

#[test]
fn other_device_strings_stay_in_the_stream() {
    assert_eq!(scan(&[b"\x1bPq#0\x1b\\"]), vec![bytes(b"\x1bPq#0\x1b\\")]);
}

#[test]
fn an_unterminated_status_query_is_released() {
    let long = [b"\x1bP$q".as_slice(), &[b'a'; 64]].concat();
    assert_eq!(scan(&[&long]), vec![bytes(&long)]);
}

const _: () = {
    assert!(MAX_STATUS < MAX_MARK);
    assert!(MAX_MARK < MAX_GRAPHICS);
};

#[test]
fn sequence_caps_are_exposed_and_ordered() {
    assert_eq!(MAX_MARK, 64 * 1024);
    assert_eq!(MAX_GRAPHICS, 2 * 1024 * 1024);
    assert_eq!(MAX_STATUS, 32);
}

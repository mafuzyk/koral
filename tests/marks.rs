use koral::marks::{Mark, Piece, Scanner};

#[derive(Debug, PartialEq, Eq)]
enum Out {
    Bytes(Vec<u8>),
    Mark(Mark),
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
        });
    }
    out
}

fn bytes(text: &[u8]) -> Out {
    Out::Bytes(text.to_vec())
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
    assert_eq!(scan(&[b"\x1b]133;A\x07"]), vec![Out::Mark(Mark::PromptStart)]);
}

#[test]
fn command_start_decodes_the_command_line() {
    assert_eq!(
        scan(&[b"\x1b]133;C;cmdline_url=echo%20%22ol%C3%A1%22%0Als\x1b\\"]),
        vec![Out::Mark(Mark::CommandStart(Some("echo \"olá\"\nls".to_string())))]
    );
}

#[test]
fn command_start_without_a_command_line_is_valid() {
    assert_eq!(scan(&[b"\x1b]133;C\x1b\\"]), vec![Out::Mark(Mark::CommandStart(None))]);
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
    assert_eq!(scan(&[b"\x1b]133;D;130\x1b\\"]), vec![Out::Mark(Mark::CommandEnd(Some(130)))]);
}

#[test]
fn command_end_without_a_status_is_valid() {
    assert_eq!(scan(&[b"\x1b]133;D\x1b\\"]), vec![Out::Mark(Mark::CommandEnd(None))]);
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
    assert_eq!(scan(&[b"\x1b]7;file:///tmp\x1b\\"]), vec![Out::Mark(Mark::Cwd("/tmp".to_string()))]);
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
        vec![bytes(b"x"), Out::Mark(Mark::CommandEnd(Some(7))), bytes(b"y")]
    );
}

#[test]
fn a_lone_escape_at_a_chunk_end_is_held_until_it_can_be_decided() {
    assert_eq!(scan(&[b"a\x1b", b"[0m"]), vec![bytes(b"a\x1b[0m")]);
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
        Piece::Mark(_) => {}
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
    scanner.feed(&endless, &mut |piece| found |= matches!(piece, Piece::Bytes(bytes) if bytes.len() > 1024));
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
    Out::Mark(Mark::Notify { title: title.to_string(), body: body.to_string() })
}

#[test]
fn osc_777_is_a_notification() {
    assert_eq!(scan(&[b"\x1b]777;notify;Build;done; ok\x1b\\"]), vec![notify("Build", "done; ok")]);
}

#[test]
fn osc_9_is_a_notification() {
    assert_eq!(scan(&[b"\x1b]9;done\x07"]), vec![notify("", "done")]);
}

#[test]
fn osc9_progress_is_not_a_notification() {
    let text = b"\x1b]9;4;1;50\x07";
    assert_eq!(scan(&[text]), vec![bytes(text)]);
}

#[test]
fn a_notification_split_across_reads_is_still_found() {
    assert_eq!(scan(&[b"\x1b]77", b"7;noti", b"fy;a;b\x07"]), vec![notify("a", "b")]);
}

#[test]
fn an_unknown_777_stays_in_the_stream() {
    let text = b"\x1b]777;foo;a\x07";
    assert_eq!(scan(&[text]).into_iter().flat_map(|out| match out {
        Out::Bytes(part) => part,
        Out::Mark(_) => panic!("not a mark"),
    }).collect::<Vec<u8>>(), text);
}

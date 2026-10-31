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

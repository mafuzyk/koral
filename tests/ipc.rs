use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use koral::ipc::{
    BlockEvent, BlockInfo, BlockSelector, PaneInfo, Request, Server, TabInfo, TabKind, decode,
    encode, json_error, json_string, parse_block_selector, parse_cli, render_block_output_json,
    render_blocks_json, render_blocks_text, render_event_ndjson, render_list_json,
    render_list_text, send, watch,
};
use koral::splits::{Axis, Dir};

#[test]
fn wire_encode_decode_round_trips_all_commands() {
    let cases = [
        Request::NewTab {
            cwd: Some(PathBuf::from("/tmp/a\tb")),
            title: Some("Line\nTitle\\One".to_string()),
            command: Some((
                "fish".to_string(),
                vec!["-c".to_string(), "echo hi".to_string()],
            )),
        },
        Request::Split {
            axis: Axis::Horizontal,
            cwd: Some(PathBuf::from("/home/mafu")),
        },
        Request::Split {
            axis: Axis::Vertical,
            cwd: None,
        },
        Request::ClosePane { pane: None },
        Request::ClosePane { pane: Some(9) },
        Request::CloseTab,
        Request::Focus(Dir::Left),
        Request::Focus(Dir::Down),
        Request::Resize(Dir::Right),
        Request::Resize(Dir::Up),
        Request::SetTitle {
            pane: None,
            title: "Tab 1\tHello".to_string(),
        },
        Request::SetTitle {
            pane: Some(4),
            title: "pane:literal".to_string(),
        },
        Request::SetTheme("catppuccin-mocha".to_string()),
        Request::OpenSettings,
        Request::Reload,
        Request::SendText {
            pane: None,
            text: "ls -la\n".to_string(),
        },
        Request::SendText {
            pane: Some(12),
            text: "echo ok\n".to_string(),
        },
        Request::List,
        Request::ListJson,
        Request::ListBlocks {
            pane: None,
            json: false,
        },
        Request::ListBlocks {
            pane: Some(3),
            json: true,
        },
        Request::GetBlockOutput {
            pane: Some(4),
            selector: BlockSelector::Id(9),
            json: true,
        },
        Request::GetBlockOutput {
            pane: None,
            selector: BlockSelector::Offset(2),
            json: false,
        },
        Request::GetBlockOutput {
            pane: Some(1),
            selector: BlockSelector::LastFailed,
            json: false,
        },
        Request::Watch { pane: Some(2) },
        Request::SaveSession("work-1".to_string()),
        Request::RestoreSession("work-1".to_string()),
        Request::ListSessions,
        Request::ListSessionsJson,
    ];
    for req in cases {
        let line = encode(&req);
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
        assert_eq!(decode(&line).expect("decode"), req);
    }
}

#[test]
fn cli_parser_handles_socket_flag_and_subcommands() {
    let argv: Vec<String> = [
        "--socket",
        "/tmp/x.sock",
        "new-tab",
        "--cwd",
        "/tmp",
        "--title",
        "Logs",
        "-e",
        "tail",
        "-f",
        "/tmp/a",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    let (sock, req) = parse_cli(&argv).unwrap();
    assert_eq!(sock, Some(PathBuf::from("/tmp/x.sock")));
    assert_eq!(
        req,
        Request::NewTab {
            cwd: Some(PathBuf::from("/tmp")),
            title: Some("Logs".to_string()),
            command: Some((
                "tail".to_string(),
                vec!["-f".to_string(), "/tmp/a".to_string()]
            )),
        }
    );

    let (sock2, req2) =
        parse_cli(&["--socket=/tmp/y.sock".into(), "split".into(), "down".into()]).unwrap();
    assert_eq!(sock2, Some(PathBuf::from("/tmp/y.sock")));
    assert_eq!(
        req2,
        Request::Split {
            axis: Axis::Vertical,
            cwd: None
        }
    );

    assert_eq!(parse_cli(&["list".into()]).unwrap().1, Request::List);
    assert!(parse_cli(&["nope".into()]).is_err());
    assert!(parse_cli(&[]).is_err());
}

#[test]
fn unix_socket_server_replies_cleans_up_and_sets_0600_mode() {
    let sock = std::env::temp_dir().join(format!("koral-ipc-test-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&sock);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen_cb = seen.clone();

    let server = Server::bind_with(sock.clone(), move |req, reply| {
        seen_cb.lock().unwrap().push(req.clone());
        let text = match req {
            Request::List => "OK\ntab 0 pane 0\n".to_string(),
            Request::SetTheme(id) if id == "missing" => "ERR unknown theme\n".to_string(),
            _ => "OK\n".to_string(),
        };
        let _ = reply.send(text);
    })
    .expect("bind ipc test socket");

    let mode = std::fs::metadata(&sock).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);

    let out = send(Some(sock.clone()), &Request::List).unwrap();
    assert_eq!(out, "tab 0 pane 0");

    let err = send(
        Some(sock.clone()),
        &Request::SetTheme("missing".to_string()),
    )
    .unwrap_err();
    assert_eq!(err, "unknown theme");

    assert_eq!(seen.lock().unwrap().len(), 2);

    drop(server);
    assert!(!sock.exists());
}

#[test]
fn json_string_escapes_quotes_backslashes_and_control_chars() {
    assert_eq!(json_string("plain utf8 ✓"), "\"plain utf8 ✓\"");
    assert_eq!(
        json_string("a\"b\\c\nd\re\tf\x00\x1b"),
        "\"a\\\"b\\\\c\\nd\\re\\tf\\u0000\\u001b\""
    );
    assert_eq!(
        json_error("bad \"socket\""),
        "{\"version\":1,\"error\":\"bad \\\"socket\\\"\"}"
    );
}

#[test]
fn cli_parser_handles_json_flag() {
    assert_eq!(
        parse_cli(&["--json".into(), "list".into()]).unwrap(),
        (None, Request::ListJson)
    );
    assert_eq!(
        parse_cli(&[
            "--socket".into(),
            "/tmp/k.sock".into(),
            "--json".into(),
            "list".into()
        ])
        .unwrap(),
        (Some(PathBuf::from("/tmp/k.sock")), Request::ListJson)
    );
    assert_eq!(
        parse_cli(&[
            "--json".into(),
            "--socket=/tmp/k.sock".into(),
            "list".into()
        ])
        .unwrap(),
        (Some(PathBuf::from("/tmp/k.sock")), Request::ListJson)
    );
    assert_eq!(
        parse_cli(&["list".into(), "--json".into()]).unwrap(),
        (None, Request::ListJson)
    );
    let err = parse_cli(&["--json".into(), "set-theme".into(), "dracula".into()]).unwrap_err();
    assert!(err.contains("--json is not supported"), "{err}");

    assert_eq!(
        parse_cli(&["save-session".into(), "work".into()])
            .unwrap()
            .1,
        Request::SaveSession("work".to_string())
    );
    assert_eq!(
        parse_cli(&["restore-session".into(), "work".into()])
            .unwrap()
            .1,
        Request::RestoreSession("work".to_string())
    );
    assert_eq!(
        parse_cli(&["list-sessions".into()]).unwrap().1,
        Request::ListSessions
    );
    assert_eq!(
        parse_cli(&["list-sessions".into(), "--json".into()])
            .unwrap()
            .1,
        Request::ListSessionsJson
    );
    assert!(parse_cli(&["save-session".into(), "../bad".into()]).is_err());
}

#[test]
fn render_list_matches_exact_text_and_versioned_json() {
    assert_eq!(render_list_text(&[]), "");
    assert_eq!(render_list_json(&[]), "{\"version\":1,\"tabs\":[]}");

    let tabs = vec![
        TabInfo {
            index: 0,
            active: false,
            kind: TabKind::Settings,
        },
        TabInfo {
            index: 1,
            active: true,
            kind: TabKind::Terminal {
                title: "~/proj \"koral\"".to_string(),
                active_pane: 4,
                panes: vec![
                    PaneInfo {
                        id: 3,
                        active: false,
                        title: "logs\t1".to_string(),
                    },
                    PaneInfo {
                        id: 4,
                        active: true,
                        title: "~/proj \"koral\"".to_string(),
                    },
                ],
            },
        },
    ];

    assert_eq!(
        render_list_text(&tabs),
        "\
tab 0  settings
tab 1* panes=2 active_pane=4 title=~/proj \"koral\"
  pane 3  title=logs\t1
  pane 4* title=~/proj \"koral\"
"
    );
    assert_eq!(
        render_list_json(&tabs),
        "{\"version\":1,\"tabs\":[{\"index\":0,\"active\":false,\"kind\":\"settings\"},{\"index\":1,\"active\":true,\"kind\":\"terminal\",\"title\":\"~/proj \\\"koral\\\"\",\"active_pane\":4,\"panes\":[{\"id\":3,\"active\":false,\"title\":\"logs\\t1\"},{\"id\":4,\"active\":true,\"title\":\"~/proj \\\"koral\\\"\"}]}]}"
    );
}

#[test]
fn pane_selectors_and_list_blocks_parse_and_render() {
    assert_eq!(
        parse_cli(&["send-text".into(), "--pane".into(), "7".into(), "hi".into()])
            .unwrap()
            .1,
        Request::SendText {
            pane: Some(7),
            text: "hi".to_string()
        }
    );
    assert_eq!(
        parse_cli(&["set-title".into(), "--pane=3".into(), "build".into()])
            .unwrap()
            .1,
        Request::SetTitle {
            pane: Some(3),
            title: "build".to_string()
        }
    );
    assert_eq!(
        parse_cli(&["close-pane".into(), "--pane".into(), "5".into()])
            .unwrap()
            .1,
        Request::ClosePane { pane: Some(5) }
    );
    assert_eq!(
        parse_cli(&[
            "list-blocks".into(),
            "--pane".into(),
            "2".into(),
            "--json".into()
        ])
        .unwrap()
        .1,
        Request::ListBlocks {
            pane: Some(2),
            json: true
        }
    );
    assert!(parse_cli(&["list-blocks".into(), "--pane".into(), "abc".into()]).is_err());

    let blocks = vec![BlockInfo {
        id: 1,
        pane_id: 2,
        command: Some("cargo test".to_string()),
        cwd: Some("/home/mafu/Projetos/koral".to_string()),
        status: Some(0),
        folded: false,
    }];
    assert_eq!(
        render_blocks_text(&blocks),
        "block 1 pane=2 status=0 folded=false cwd=/home/mafu/Projetos/koral cmd=cargo test\n"
    );
    assert_eq!(
        render_blocks_json(&blocks),
        "{\"version\":1,\"blocks\":[{\"id\":1,\"pane_id\":2,\"command\":\"cargo test\",\"cwd\":\"/home/mafu/Projetos/koral\",\"status\":0,\"folded\":false}]}"
    );
}

#[test]
fn get_block_output_and_ndjson_watch_stream_work() {
    assert_eq!(
        parse_cli(&[
            "get-block-output".into(),
            "--pane".into(),
            "2".into(),
            "--block".into(),
            "7".into(),
            "--json".into(),
        ])
        .unwrap()
        .1,
        Request::GetBlockOutput {
            pane: Some(2),
            selector: BlockSelector::Id(7),
            json: true,
        }
    );
    assert_eq!(
        parse_cli(&["get-block-output".into(), "--block=@-2".into()])
            .unwrap()
            .1,
        Request::GetBlockOutput {
            pane: None,
            selector: BlockSelector::Offset(2),
            json: false,
        }
    );
    assert_eq!(
        parse_cli(&["get-block-output".into(), "--block".into(), "@err".into()])
            .unwrap()
            .1,
        Request::GetBlockOutput {
            pane: None,
            selector: BlockSelector::LastFailed,
            json: false,
        }
    );
    assert_eq!(parse_block_selector("@-1"), Ok(BlockSelector::Offset(1)));
    assert_eq!(parse_block_selector("@err"), Ok(BlockSelector::LastFailed));
    assert_eq!(parse_block_selector("@42"), Ok(BlockSelector::Id(42)));
    assert!(parse_block_selector("@-0").is_err());
    assert_eq!(
        parse_cli(&["watch".into(), "--pane=4".into(), "--json".into()])
            .unwrap()
            .1,
        Request::Watch { pane: Some(4) }
    );
    assert_eq!(
        render_block_output_json(2, 7, "ok\n"),
        "{\"version\":1,\"pane_id\":2,\"block_id\":7,\"output\":\"ok\\n\"}"
    );

    let ev = BlockEvent {
        pane_id: 4,
        kind: "command_end".to_string(),
        block_id: Some(7),
        command: Some("cargo test".to_string()),
        cwd: Some("/tmp".to_string()),
        status: Some(0),
    };
    let ev_json = render_event_ndjson(&ev);
    assert_eq!(
        ev_json,
        "{\"version\":1,\"event\":\"command_end\",\"pane_id\":4,\"block_id\":7,\"command\":\"cargo test\",\"cwd\":\"/tmp\",\"status\":0}"
    );

    let sock = std::env::temp_dir().join(format!("koral-ipc-watch-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&sock);
    let server = Server::bind_with(sock.clone(), |_, tx| {
        let _ = tx.send("OK\n".to_string());
    })
    .unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen_bg = seen.clone();
    let sock_bg = sock.clone();
    let handle = std::thread::spawn(move || {
        watch(Some(sock_bg), &Request::Watch { pane: Some(4) }, |line| {
            seen_bg.lock().unwrap().push(line.to_string());
        })
        .unwrap();
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    server.publish(5, "ignored");
    while server.publish(4, &ev_json) == 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "watcher did not subscribe in time"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    while seen.lock().unwrap().is_empty() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    drop(server);
    let _ = handle.join();
    assert_eq!(seen.lock().unwrap().as_slice(), [ev_json]);
}

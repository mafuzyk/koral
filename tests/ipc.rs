use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use koral::ipc::{Request, Server, decode, encode, parse_cli, send};
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
        Request::ClosePane,
        Request::CloseTab,
        Request::Focus(Dir::Left),
        Request::Focus(Dir::Down),
        Request::Resize(Dir::Right),
        Request::Resize(Dir::Up),
        Request::SetTitle("Tab 1\tHello".to_string()),
        Request::SetTheme("catppuccin-mocha".to_string()),
        Request::OpenSettings,
        Request::Reload,
        Request::SendText("ls -la\n".to_string()),
        Request::List,
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

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use koral::splits::Axis;
use koral::workspace::{
    SavedNode, SavedSession, SavedTab, VERSION, decode_json, encode_json, list, load,
    render_sessions_json, render_sessions_text, save, validate_name,
};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "koral-workspace-test-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn save_load_and_list_round_trips_nested_splits_and_0600_mode() {
    let dir = scratch("roundtrip");
    let session = SavedSession {
        version: VERSION,
        name: "dev-env_1".to_string(),
        active_tab: 1,
        tabs: vec![
            SavedTab {
                title: Some("editor".to_string()),
                layout: SavedNode::Pane {
                    cwd: Some("/home/mafu/Projetos/koral".to_string()),
                    active: true,
                },
            },
            SavedTab {
                title: None,
                layout: SavedNode::Split {
                    axis: Axis::Horizontal,
                    ratio: 0.625,
                    first: Box::new(SavedNode::Pane {
                        cwd: Some("/tmp/left \"quoted\"".to_string()),
                        active: false,
                    }),
                    second: Box::new(SavedNode::Split {
                        axis: Axis::Vertical,
                        ratio: 0.35,
                        first: Box::new(SavedNode::Pane {
                            cwd: Some("/tmp/top".to_string()),
                            active: true,
                        }),
                        second: Box::new(SavedNode::Pane {
                            cwd: None,
                            active: false,
                        }),
                    }),
                },
            },
        ],
    };

    let path = save(&dir, &session).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    assert_eq!(load(&dir, "dev-env_1").unwrap(), session);
    assert_eq!(list(&dir).unwrap(), vec!["dev-env_1".to_string()]);

    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(!raw.contains("scrollback"));
    assert!(!raw.contains("command"));

    assert_eq!(
        render_sessions_text(&["alpha".into(), "beta".into()]),
        "alpha\nbeta\n"
    );
    assert_eq!(
        render_sessions_json(&["alpha".into(), "beta".into()]),
        "{\"version\":1,\"sessions\":[\"alpha\",\"beta\"]}"
    );
}

#[test]
fn rejects_unsafe_session_names_and_corrupt_json() {
    for bad in [
        "",
        ".",
        "..",
        "../evil",
        "a/b",
        "with space",
        &"x".repeat(65),
    ] {
        assert!(validate_name(bad).is_err(), "{bad:?} must fail validation");
    }
    assert!(decode_json("{not valid").is_err());
    assert!(decode_json("{\"version\":99,\"name\":\"ok\",\"active_tab\":0,\"tabs\":[]}").is_err());
    let round = encode_json(&SavedSession {
        version: VERSION,
        name: "a.b".to_string(),
        active_tab: 0,
        tabs: vec![],
    });
    assert_eq!(decode_json(&round).unwrap().name, "a.b");
}

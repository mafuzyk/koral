use std::path::PathBuf;

use koral::config::{Config, append, log_path, migrate, parse, path, set};
use koral::input::{Action, BindKey, Keybind};
use winit::keyboard::ModifiersState;

fn config(source: &str) -> Config {
    parse(source).0
}

fn errors(source: &str) -> Vec<String> {
    parse(source).1
}

#[test]
fn an_empty_file_gives_the_defaults() {
    let config = config("");
    assert_eq!(config, Config::default());
    assert_eq!(config.font, "IosevkaTerm Nerd Font Mono");
    assert_eq!(config.font_size, 13.5);
    assert_eq!(config.opacity, 0.8);
    assert_eq!(config.padding, 8.0);
    assert_eq!(config.shell, None);
}

#[test]
fn present_mode_defaults_to_auto_and_rejects_unknown_modes() {
    use koral::config::PresentModePolicy;
    assert_eq!(config("").present_mode, PresentModePolicy::Auto);
    for (name, policy) in [
        ("auto", PresentModePolicy::Auto),
        ("fifo", PresentModePolicy::Fifo),
        ("mailbox", PresentModePolicy::Mailbox),
        ("immediate", PresentModePolicy::Immediate),
    ] {
        assert_eq!(config(&set("", "present_mode", name)).present_mode, policy);
    }
    assert!(!errors(&set("", "present_mode", "fastest")).is_empty());
}

#[test]
fn system_title_bar_is_opt_in_and_round_trips() {
    assert!(!config("").window_decorations);
    let enabled = set("", "window_decorations", "on");
    assert!(config(&enabled).window_decorations);
    let disabled = set(&enabled, "window_decorations", "off");
    assert!(!config(&disabled).window_decorations);
}

#[test]
fn statements_set_their_values() {
    let config = config("font Fira Code\nfont_size 12\nopacity 0.5\npadding 4\nshell fish\n");
    assert_eq!(config.font, "Fira Code");
    assert_eq!(config.font_size, 12.0);
    assert_eq!(config.opacity, 0.5);
    assert_eq!(config.padding, 4.0);
    assert_eq!(config.shell.as_deref(), Some("fish"));
}

#[test]
fn comments_and_blank_lines_are_ignored() {
    assert_eq!(errors("# settings\n\nopacity 0.5\n"), Vec::<String>::new());
    assert_eq!(config("# comment\nopacity 0.5\n").opacity, 0.5);
}

#[test]
fn hash_inside_value_is_text_not_inline_comment() {
    assert_eq!(config("font a#b\n").font, "a#b");
    assert_eq!(config("font \"#leading\"\n").font, "#leading");
}

#[test]
fn unknown_statements_are_errors_with_a_line_number() {
    let found = errors("\nwhale\n");
    assert_eq!(found.len(), 1);
    assert!(found[0].starts_with("line 2:"), "{found:?}");
}

#[test]
fn unknown_keys_are_errors() {
    let errs = errors("nope 1\n");
    assert_eq!(errs.len(), 1);
    assert!(errs[0].contains("unknown setting 'nope'"), "{errs:?}");
}

#[test]
fn out_of_range_values_keep_the_default() {
    let source = "opacity 7\nfont_size 1\npadding -3\n";
    assert_eq!(config(source), Config::default());
    assert_eq!(errors(source).len(), 3);
}

#[test]
fn non_numeric_values_keep_the_default() {
    assert_eq!(config("opacity nan\n").opacity, 0.8);
    assert_eq!(config("opacity lots\n").opacity, 0.8);
}

#[test]
fn an_empty_font_keeps_the_default() {
    assert_eq!(config("font \"\"\n"), Config::default());
}

#[test]
fn set_changes_only_that_line() {
    let source = "# mine\nopacity 0.5\n\nfont_size 12\n";
    assert_eq!(
        set(source, "font_size", "14"),
        "# mine\nopacity 0.5\n\nfont_size 14\n"
    );
}

#[test]
fn set_appends_a_missing_setting() {
    assert_eq!(set("# mine\n", "opacity", "0.6"), "# mine\nopacity 0.6\n");
}

#[test]
fn set_adds_the_final_newline_a_file_was_missing() {
    assert_eq!(set("# mine", "opacity", "0.6"), "# mine\nopacity 0.6\n");
}

#[test]
fn set_round_trips_spaces_hashes_and_quotes() {
    assert_eq!(set("", "font", "Fira Code"), "font Fira Code\n");
    assert_eq!(config(&set("", "font", "Fira Code")).font, "Fira Code");
    assert_eq!(config(&set("", "font", "Font #1")).font, "Font #1");
    assert_eq!(set("", "font", "#hash"), "font \"#hash\"\n");
    assert_eq!(config(&set("", "font", "#hash")).font, "#hash");
    assert_eq!(config(&set("", "font", "a\"b\\c")).font, "a\"b\\c");
    assert_eq!(config(&set("", "font", "\"quoted\"")).font, "\"quoted\"");
}

#[test]
fn set_ignores_the_key_inside_comments() {
    let source = "# opacity 0.1\n";
    assert_eq!(
        set(source, "opacity", "0.6"),
        "# opacity 0.1\nopacity 0.6\n"
    );
}

#[test]
fn set_updates_the_last_duplicate_because_it_wins() {
    let source = "opacity 0.1\nopacity 0.2\n";
    let result = set(source, "opacity", "0.6");
    assert_eq!(config(&result).opacity, 0.6);
    assert!(result.starts_with("opacity 0.1\n"));
}

#[test]
fn matugen_is_off_until_turned_on() {
    assert!(!config("").matugen);
    assert!(config("matugen on").matugen);
    assert!(!config("matugen off").matugen);
}

#[test]
fn matugen_takes_only_on_or_off() {
    assert_eq!(errors("matugen yes").len(), 1);
}

#[test]
fn shell_integration_defaults_on_and_toggles() {
    assert!(config("").shell_integration);
    assert!(!config("shell_integration off\n").shell_integration);
    assert!(config("shell_integration on\n").shell_integration);
    assert_eq!(errors("shell_integration auto\n").len(), 1);
}

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("koral-migrate-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn the_config_file_is_koral_conf() {
    assert!(path().unwrap().ends_with("koral/koral.conf"));
}

#[test]
fn the_log_is_koral_log_next_to_the_config() {
    assert!(log_path().unwrap().ends_with("koral/koral.log"));
}

#[test]
fn the_log_keeps_one_timestamped_line_per_event() {
    let dir = folder("log");
    let path = dir.join("koral.log");
    append(&path, "koral.conf line 2: bad").unwrap();
    append(&path, "cannot start nope").unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].starts_with('['), "{:?}", lines[0]);
    assert!(
        lines[0].ends_with("koral.conf line 2: bad"),
        "{:?}",
        lines[0]
    );
    assert!(lines[1].ends_with("cannot start nope"), "{:?}", lines[1]);
}

#[test]
fn migrate_converts_config_sharky_without_modifying_it() {
    let dir = folder("sharky-full");
    let legacy = "\
// user settings
polyp font \"Fira Code\"
polyp font_size 14 // keep note

polyp theme \"#custom\"
bind ctrl+shift+n new_tab
unknown_directive foo
";
    let old_path = dir.join("config.sharky");
    std::fs::write(&old_path, legacy).unwrap();

    let migrated_from = migrate(&dir).unwrap();
    assert_eq!(migrated_from.as_deref(), Some(old_path.as_path()));

    let converted = std::fs::read_to_string(dir.join("koral.conf")).unwrap();
    assert_eq!(
        converted,
        "\
# user settings
font Fira Code
# keep note
font_size 14

theme \"#custom\"
keybind ctrl+shift+n=new_tab
# not migrated: unknown_directive foo
"
    );
    // Source file stays byte-identical.
    assert_eq!(std::fs::read_to_string(&old_path).unwrap(), legacy);

    let (parsed, errs) = parse(&converted);
    assert_eq!(parsed.font, "Fira Code");
    assert_eq!(parsed.font_size, 14.0);
    assert_eq!(parsed.bindings.len(), 1);
    // Only the `#custom` theme value is invalid; the rest migrated cleanly.
    assert_eq!(errs.len(), 1, "{errs:?}");
}

#[test]
fn migrate_uses_config_koral_only_when_config_sharky_is_absent() {
    let dir = folder("fallback-koral");
    std::fs::write(dir.join("config.koral"), "polyp font_size 14\n").unwrap();
    let migrated_from = migrate(&dir).unwrap();
    assert_eq!(migrated_from, Some(dir.join("config.koral")));
    assert_eq!(
        std::fs::read_to_string(dir.join("koral.conf")).unwrap(),
        "font_size 14\n"
    );
    assert!(dir.join("config.koral").exists());

    let both = folder("prefer-sharky");
    std::fs::write(both.join("config.koral"), "polyp font_size 14\n").unwrap();
    std::fs::write(both.join("config.sharky"), "polyp font_size 18\n").unwrap();
    let from_both = migrate(&both).unwrap();
    assert_eq!(from_both, Some(both.join("config.sharky")));
    assert_eq!(
        std::fs::read_to_string(both.join("koral.conf")).unwrap(),
        "font_size 18\n"
    );
}

#[test]
fn migrate_never_overwrites_existing_koral_conf() {
    let dir = folder("existing");
    std::fs::write(dir.join("config.sharky"), "polyp font_size 14\n").unwrap();
    std::fs::write(dir.join("koral.conf"), "font_size 20\n").unwrap();
    assert_eq!(migrate(&dir).unwrap(), None);
    assert_eq!(
        std::fs::read_to_string(dir.join("koral.conf")).unwrap(),
        "font_size 20\n"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("config.sharky")).unwrap(),
        "polyp font_size 14\n"
    );
}

#[test]
fn migrate_with_no_files_creates_nothing() {
    let dir = folder("none");
    assert_eq!(migrate(&dir).unwrap(), None);
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
}

#[test]
fn weights_parse_and_stay_in_range() {
    assert_eq!(errors("bold_weight 950\n").len(), 1);
    assert_eq!(config("bold_weight 700\n").face().bold, 700);
}

#[test]
fn italic_style_is_oblique_or_italic() {
    assert_eq!(errors("italic_style slanted\n").len(), 1);
    assert!(!config("italic_style italic\n").face().oblique);
    assert!(config("").face().oblique);
}

#[test]
fn theme_scrollback_clipboard_and_binds_parse_and_validate() {
    let source = "\
theme dracula
scrollback 50000
clipboard_read on
keybind ctrl+shift+n=new_tab
keybind ctrl++=zoom_in
keybind ctrl+shift+,=settings
";
    let (parsed, errs) = parse(source);
    assert!(errs.is_empty(), "{errs:?}");
    assert_eq!(parsed.theme.as_deref(), Some("dracula"));
    assert_eq!(parsed.scrollback, 50_000);
    assert!(parsed.clipboard_read);
    assert_eq!(
        parsed.bindings,
        vec![
            Keybind {
                mods: ModifiersState::CONTROL | ModifiersState::SHIFT,
                key: BindKey::Char('n'),
                action: Some(Action::NewTab),
            },
            Keybind {
                mods: ModifiersState::CONTROL,
                key: BindKey::Char('+'),
                action: Some(Action::ZoomIn),
            },
            Keybind {
                mods: ModifiersState::CONTROL | ModifiersState::SHIFT,
                key: BindKey::Char(','),
                action: Some(Action::OpenSettings),
            },
        ]
    );

    assert_eq!(errors("theme nonexistent\n").len(), 1);
    assert_eq!(errors("scrollback -5\n").len(), 1);
    assert_eq!(errors("scrollback 99999999\n").len(), 1);
    assert_eq!(errors("clipboard_read maybe\n").len(), 1);
    assert_eq!(errors("keybind ctrl+shift+x=nope\n").len(), 1);
    assert_eq!(errors("keybind ctrl+shift+x\n").len(), 1);
    assert!(set(source, "opacity", "0.75").contains("keybind ctrl+shift+n=new_tab"));
}

#[test]
fn ligatures_and_font_features_parse_and_validate() {
    assert!(config("").ligatures);
    assert!(config("").font_features.is_empty());

    let parsed = config("ligatures off\nfont_features +zero -calt ss01=1 cv02\n");
    assert!(!parsed.ligatures);
    assert_eq!(
        parsed.font_features,
        vec![(*b"zero", 1), (*b"calt", 0), (*b"ss01", 1), (*b"cv02", 1)]
    );
    let face = parsed.face();
    assert!(!face.ligatures);
    assert_eq!(face.features, parsed.font_features);

    assert!(
        config("font_features +zero\nfont_features none\n")
            .font_features
            .is_empty()
    );
    assert_eq!(errors("ligatures maybe\n").len(), 1);
    assert_eq!(errors("font_features badtag\n").len(), 1);
    assert_eq!(errors("font_features ss01=abc\n").len(), 1);
}

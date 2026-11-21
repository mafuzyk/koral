use std::time::{Duration, UNIX_EPOCH};

use koral::matugen::{TEMPLATE, add_template, diff_stamps, on_path};
use koral::theme::{Theme, hex};

const NEW: &str = "[templates.koral]\ninput_path = \"/c/koral/matugen.template\"\noutput_path = \"/c/koral/matugen.colors\"\n";

fn added(source: &str) -> Option<String> {
    add_template(source, "/c/koral/matugen.template".as_ref(), "/c/koral/matugen.colors".as_ref())
}

#[test]
fn the_template_is_added_after_what_the_file_already_has() {
    let updated = added("[config]\nversion_check = false\n").unwrap();
    assert_eq!(updated, format!("[config]\nversion_check = false\n\n{NEW}"));
}

#[test]
fn a_file_without_a_final_newline_keeps_its_last_line_whole() {
    let updated = added("[config]\nversion_check = false").unwrap();
    assert!(updated.starts_with("[config]\nversion_check = false\n\n[templates.koral]"));
}

#[test]
fn a_second_toggle_adds_nothing() {
    let once = added("[config]\n").unwrap();
    assert_eq!(added(&once), None);
}

#[test]
fn quotes_in_a_path_stay_inside_the_toml_string() {
    let updated = add_template("", "/a\"b/t".as_ref(), "/c/o".as_ref()).unwrap();
    assert!(updated.contains(r#"input_path = "/a\"b/t""#));
}

#[test]
fn only_an_executable_matugen_on_the_path_counts() {
    use std::os::unix::fs::PermissionsExt;

    let dir = std::env::temp_dir().join(format!("koral-matugen-path-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = Some(dir.as_os_str());
    assert!(!on_path(path));

    let program = dir.join("matugen");
    std::fs::write(&program, "").unwrap();
    std::fs::set_permissions(&program, PermissionsExt::from_mode(0o644)).unwrap();
    assert!(!on_path(path));

    std::fs::set_permissions(&program, PermissionsExt::from_mode(0o755)).unwrap();
    assert!(on_path(path));
    assert!(!on_path(None));
}

#[test]
fn the_template_sets_every_colour_of_the_theme() {
    let colors = TEMPLATE.lines().map(|line| format!("{} #010203\n", line.split(' ').next().unwrap()));
    let theme = Theme::with_colors(&colors.collect::<String>());
    let color = hex("#010203");
    assert!(theme.ansi.iter().all(|ansi| Some(*ansi) == color));
    for slot in [theme.background, theme.foreground, theme.cursor, theme.accent, theme.success, theme.failure, theme.muted] {
        assert_eq!(Some(slot), color);
    }
}

#[test]
fn diff_stamps_separates_theme_changes_from_config_changes() {
    let t0 = Some(UNIX_EPOCH + Duration::from_secs(10));
    let t1 = Some(UNIX_EPOCH + Duration::from_secs(20));
    let base = [t0, t0, t0];
    assert_eq!(diff_stamps(&base, &base), (false, false));
    assert_eq!(diff_stamps(&base, &[t1, t0, t0]), (true, false));
    assert_eq!(diff_stamps(&base, &[t0, t1, t0]), (true, false));
    assert_eq!(diff_stamps(&base, &[t0, t0, t1]), (false, true));
    assert_eq!(diff_stamps(&base, &[t1, t0, t1]), (true, true));
}

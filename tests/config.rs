use koral::config::{Config, parse, set};

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
fn statements_set_their_values() {
    let config = config("polyp font \"Fira Code\"\npolyp font_size 12\npolyp opacity 0.5\npolyp padding 4\npolyp shell fish\n");
    assert_eq!(config.font, "Fira Code");
    assert_eq!(config.font_size, 12.0);
    assert_eq!(config.opacity, 0.5);
    assert_eq!(config.padding, 4.0);
    assert_eq!(config.shell.as_deref(), Some("fish"));
}

#[test]
fn comments_and_blank_lines_are_ignored() {
    assert_eq!(errors("// settings\n\npolyp opacity 0.5 // see-through\n"), Vec::<String>::new());
    assert_eq!(config("polyp opacity 0.5 // see-through\n").opacity, 0.5);
}

#[test]
fn a_comment_marker_inside_quotes_is_text() {
    assert_eq!(config("polyp font \"a//b\"\n").font, "a//b");
}

#[test]
fn unknown_statements_are_errors_with_a_line_number() {
    let found = errors("\nwhale font x\n");
    assert_eq!(found.len(), 1);
    assert!(found[0].starts_with("line 2:"), "{found:?}");
}

#[test]
fn unknown_keys_are_errors() {
    assert_eq!(errors("polyp nope 1\n").len(), 1);
}

#[test]
fn out_of_range_values_keep_the_default() {
    let source = "polyp opacity 7\npolyp font_size 1\npolyp padding -3\n";
    assert_eq!(config(source), Config::default());
    assert_eq!(errors(source).len(), 3);
}

#[test]
fn non_numeric_values_keep_the_default() {
    assert_eq!(config("polyp opacity nan\n").opacity, 0.8);
    assert_eq!(config("polyp opacity lots\n").opacity, 0.8);
}

#[test]
fn an_empty_font_keeps_the_default() {
    assert_eq!(config("polyp font \"\"\n"), Config::default());
}

#[test]
fn set_changes_only_that_line() {
    let source = "// mine\npolyp opacity 0.5 // keep\n\npolyp font_size 12\n";
    assert_eq!(set(source, "font_size", "14"), "// mine\npolyp opacity 0.5 // keep\n\npolyp font_size 14\n");
}

#[test]
fn set_keeps_a_trailing_comment_on_the_changed_line() {
    assert_eq!(set("polyp opacity 0.5 // keep\n", "opacity", "0.6"), "polyp opacity 0.6 // keep\n");
}

#[test]
fn set_appends_a_missing_setting() {
    assert_eq!(set("// mine\n", "opacity", "0.6"), "// mine\npolyp opacity 0.6\n");
}

#[test]
fn set_adds_the_final_newline_a_file_was_missing() {
    assert_eq!(set("// mine", "opacity", "0.6"), "// mine\npolyp opacity 0.6\n");
}

#[test]
fn set_quotes_values_with_spaces_or_quotes() {
    assert_eq!(set("", "font", "Fira Code"), "polyp font \"Fira Code\"\n");
    assert_eq!(config(&set("", "font", "a\"b\\c")).font, "a\"b\\c");
}

#[test]
fn set_ignores_the_key_inside_comments() {
    let source = "// polyp opacity 0.1\n";
    assert_eq!(set(source, "opacity", "0.6"), "// polyp opacity 0.1\npolyp opacity 0.6\n");
}

#[test]
fn set_updates_the_last_duplicate_because_it_wins() {
    let source = "polyp opacity 0.1\npolyp opacity 0.2\n";
    let result = set(source, "opacity", "0.6");
    assert_eq!(config(&result).opacity, 0.6);
    assert!(result.starts_with("polyp opacity 0.1\n"));
}

#[test]
fn matugen_is_off_until_turned_on() {
    assert!(!config("").matugen);
    assert!(config("polyp matugen on").matugen);
    assert!(!config("polyp matugen off").matugen);
}

#[test]
fn matugen_takes_only_on_or_off() {
    assert_eq!(errors("polyp matugen yes").len(), 1);
}

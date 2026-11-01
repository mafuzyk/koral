use std::path::PathBuf;

use koral::fonts::{cached_files, files_text, key, listed_files, quick};
use koral::render::Face;

const FAMILY: &str = "Iosevka Term";

fn face() -> Face {
    Face { family: FAMILY.to_string(), weight: 500, bold: 800, bold_italic: 900, oblique: true }
}

fn files() -> Vec<PathBuf> {
    vec![PathBuf::from("/fonts/a.ttf"), PathBuf::from("/fonts/b c.ttf")]
}

#[test]
fn the_list_reads_back_for_its_family() {
    assert_eq!(cached_files(&files_text(FAMILY, &files()), FAMILY), Some(files()));
}

#[test]
fn a_list_for_another_font_is_ignored() {
    assert_eq!(cached_files(&files_text("Fira Code", &files()), FAMILY), None);
}

#[test]
fn an_empty_list_is_no_list() {
    assert_eq!(cached_files(&files_text(FAMILY, &[]), FAMILY), None);
    assert_eq!(cached_files("", FAMILY), None);
}

#[test]
fn another_weight_is_another_list() {
    assert_ne!(key(&face()), key(&Face { bold: 700, ..face() }));
    assert_ne!(key(&face()), key(&Face { oblique: false, ..face() }));
}

#[test]
fn no_cache_file_means_the_slow_path() {
    assert!(quick(&PathBuf::from("/nonexistent/koral-fonts"), &face()).is_none());
}

#[test]
fn a_font_that_moved_means_the_slow_path() {
    let path = std::env::temp_dir().join(format!("koral-fonts-{}", std::process::id()));
    std::fs::write(&path, files_text(&key(&face()), &files())).unwrap();
    let system = quick(&path, &face());
    std::fs::remove_file(&path).unwrap();
    assert!(system.is_none());
}

const LISTED: &str = "/f/Regular.ttf|80|0
/f/Medium.ttf|100|0
/f/MediumOblique.ttf|100|110
/f/MediumItalic.ttf|100|100
/f/ExtraBold.ttf|205|0
/f/HeavyOblique.ttf|210|110
/f/Thin.ttf|0|0
";

#[test]
fn the_font_list_keeps_only_the_styles_koral_draws() {
    let files: Vec<PathBuf> =
        ["Medium", "MediumOblique", "ExtraBold", "HeavyOblique"].map(|name| PathBuf::from(format!("/f/{name}.ttf"))).into();
    assert_eq!(listed_files(LISTED, &face()), files);
}

#[test]
fn a_variable_font_is_kept_whatever_its_weight() {
    let face = Face { weight: 400, ..face() };
    assert_eq!(listed_files("/f/Variable.ttf|[0 210]|0\n", &face), vec![PathBuf::from("/f/Variable.ttf")]);
}

#[test]
fn a_path_with_the_separator_survives() {
    assert_eq!(listed_files("/f/a|b.ttf|100|0\n", &face()), vec![PathBuf::from("/f/a|b.ttf")]);
}

#[test]
fn nothing_listed_means_the_slow_path() {
    assert!(listed_files("", &face()).is_empty());
}

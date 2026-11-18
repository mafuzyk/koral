use std::path::PathBuf;

use glyphon::fontdb::{Database, Family};
use koral::fonts::{cached_files, files_text, key, listed_files, monospaced, quick, usable};
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

fn table(tables: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut head = vec![0, 1, 0, 0];
    head.extend((tables.len() as u16).to_be_bytes());
    head.extend([0; 6]);
    let mut offset = 12 + 16 * tables.len();
    let mut body = Vec::new();
    for (tag, data) in tables {
        head.extend(*tag);
        head.extend([0; 4]);
        head.extend((offset as u32).to_be_bytes());
        head.extend((data.len() as u32).to_be_bytes());
        let padded = data.len().next_multiple_of(4);
        body.extend(data);
        body.resize(body.len() + padded - data.len(), 0);
        offset += padded;
    }
    head.extend(body);
    head
}

/// The smallest font fontdb and ttf-parser read: `i` and `M` with the given advances.
fn font(family: &str, i: u16, m: u16) -> Vec<u8> {
    styled(family, i, m, 400, false)
}

fn styled(family: &str, i: u16, m: u16, weight: u16, oblique: bool) -> Vec<u8> {
    let mut os2 = vec![0; 96];
    os2[4..6].copy_from_slice(&weight.to_be_bytes());
    os2[6..8].copy_from_slice(&5u16.to_be_bytes());
    os2[1] = 4;
    os2[62..64].copy_from_slice(&if oblique { 0x0200u16 } else { 0x0040u16 }.to_be_bytes());
    let mut head = vec![0; 54];
    head[1] = 1;
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    let mut hhea = vec![0; 36];
    hhea[1] = 1;
    hhea[35] = 3;
    let maxp = vec![0, 0, 0x50, 0, 0, 3];
    let hmtx = [0, i, m].iter().flat_map(|advance| [advance.to_be_bytes(), [0; 2]].concat()).collect();
    let mut cmap = vec![0, 0, 0, 1, 0, 0, 0, 3, 0, 0, 0, 12, 0, 0, 1, 6, 0, 0];
    cmap.extend([0; 256]);
    cmap[18 + b'i' as usize] = 1;
    cmap[18 + b'M' as usize] = 2;
    let utf16: Vec<u8> = family.encode_utf16().flat_map(u16::to_be_bytes).collect();
    let mut name = vec![0, 0, 0, 2, 0, 30];
    for id in [1u8, 6] {
        name.extend([0, 3, 0, 1, 4, 9, 0, id, 0, utf16.len() as u8, 0, 0]);
    }
    name.extend(&utf16);
    table(&[(b"OS/2", os2), (b"cmap", cmap), (b"head", head), (b"hhea", hhea), (b"hmtx", hmtx), (b"maxp", maxp), (b"name", name)])
}

fn database() -> Database {
    let mut db = Database::new();
    db.load_font_data(font("Zed Sans", 300, 800));
    db.load_font_data(font("Beta Mono", 600, 600));
    db.load_font_data(font("Alpha Term", 500, 500));
    db
}

fn asking_for(family: &str) -> Face {
    Face { family: family.to_string(), weight: 400, bold: 400, bold_italic: 400, oblique: false }
}

/// A plain monospace next to the two Nerd Font patches a terminal may meet.
fn icon_system() -> Database {
    let mut db = Database::new();
    db.load_font_data(font("DejaVu Sans Mono", 600, 600));
    db.load_font_data(font("JetBrainsMono Nerd Font", 600, 600));
    db.load_font_data(font("JetBrainsMono Nerd Font Mono", 600, 600));
    db
}

#[test]
fn a_missing_font_becomes_the_icon_font_when_one_is_installed() {
    let mut db = icon_system();
    db.set_monospace_family("DejaVu Sans Mono");
    assert_eq!(usable(&asking_for("Monocraft"), &db), asking_for("JetBrainsMono Nerd Font Mono"));
}

#[test]
fn a_proportional_font_still_becomes_the_icon_font() {
    let mut db = icon_system();
    db.load_font_data(font("Zed Sans", 300, 800));
    assert_eq!(usable(&asking_for("Zed Sans"), &db), asking_for("JetBrainsMono Nerd Font Mono"));
}

#[test]
fn an_icon_font_off_the_grid_is_not_a_substitute() {
    let mut db = Database::new();
    db.load_font_data(font("DejaVu Sans Mono", 600, 600));
    db.load_font_data(font("Symbols Nerd Font Mono", 300, 900));
    assert_eq!(usable(&asking_for("Monocraft"), &db), asking_for("DejaVu Sans Mono"));
}

fn family(faces: &[(u16, bool)]) -> Database {
    let mut db = Database::new();
    for (weight, oblique) in faces {
        db.load_font_data(styled("Vera Mono", 600, 600, *weight, *oblique));
    }
    db
}

#[test]
fn the_font_list_holds_the_fixed_width_families_in_order() {
    assert_eq!(monospaced(&database()), ["Alpha Term", "Beta Mono"]);
}

#[test]
fn an_installed_fixed_width_font_is_kept() {
    assert_eq!(usable(&asking_for("Beta Mono"), &database()), asking_for("Beta Mono"));
}

#[test]
fn a_missing_font_becomes_a_fixed_width_one_of_the_same_style() {
    assert_eq!(usable(&asking_for("Monocraft"), &database()), asking_for("Alpha Term"));
}

#[test]
fn a_proportional_font_never_reaches_the_grid() {
    assert_eq!(usable(&asking_for("Zed Sans"), &database()), asking_for("Alpha Term"));
}

#[test]
fn the_system_monospace_font_is_preferred_as_the_substitute() {
    let mut db = database();
    db.set_monospace_family("Beta Mono");
    assert_eq!(db.family_name(&Family::Monospace), "Beta Mono");
    assert_eq!(usable(&asking_for("Monocraft"), &db), asking_for("Beta Mono"));
}

#[test]
fn with_no_fixed_width_font_the_request_stays() {
    let mut db = Database::new();
    db.load_font_data(font("Zed Sans", 300, 800));
    assert_eq!(usable(&asking_for("Monocraft"), &db), asking_for("Monocraft"));
}

#[test]
fn a_family_without_the_weights_asked_for_uses_its_nearest_ones() {
    let db = family(&[(400, false), (700, false), (400, true), (700, true)]);
    let asked = Face { family: "Vera Mono".to_string(), weight: 500, bold: 800, bold_italic: 900, oblique: true };
    let got = usable(&asked, &db);
    assert_eq!((got.weight, got.bold, got.bold_italic), (400, 700, 700));
}

#[test]
fn a_font_without_the_styles_asked_for_keeps_all_its_files() {
    let output = "/fonts/a.ttf|80|0\n/fonts/b.ttf|200|0\n";
    let files = listed_files(output, &face());
    assert_eq!(files, [PathBuf::from("/fonts/a.ttf"), PathBuf::from("/fonts/b.ttf")]);
}

/// The warm-up runs off the frame, so all it must do is leave the system able to draw the sample.
#[test]
fn warming_leaves_the_sample_shapeable_again() {
    use glyphon::{Buffer, Metrics, Shaping};
    use koral::fonts::{WARM_SAMPLE, full, warm};
    use koral::render::attrs;

    let mut font_system = full(None, &face());
    if font_system.db().faces().next().is_none() {
        // cosmic-text panics with no font installed, and so would the renderer this warms for.
        return;
    }
    let installed = usable(&face(), font_system.db());
    warm(&mut font_system, &installed, 13.0);

    let pixels: f32 = 13.0 * 96.0 / 72.0;
    let metrics = Metrics::new(pixels, (pixels * 1.2).round());
    let mut buffer = Buffer::new(&mut font_system, metrics);
    buffer.set_text(WARM_SAMPLE, &attrs(&installed, false, false), Shaping::Advanced, None);
    buffer.shape_until_scroll(&mut font_system, false);
    let glyphs: usize = buffer.layout_runs().map(|run| run.glyphs.len()).sum();
    assert_eq!(glyphs, WARM_SAMPLE.chars().count(), "every character of the sample keeps a glyph");
}

use std::path::{Path, PathBuf};
use std::process::Command;

use glyphon::{FontSystem, fontdb};

use crate::render::Face;

const LOCALE: &str = "en-US";

pub fn cache_path() -> Option<PathBuf> {
    let cache = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from);
    Some(cache.or_else(|| Some(crate::config::home()?.join(".cache")))?.join("koral/fonts"))
}

pub fn files_text(family: &str, files: &[PathBuf]) -> String {
    let mut text = format!("{family}\n");
    for file in files {
        text.push_str(&format!("{}\n", file.display()));
    }
    text
}

pub fn cached_files(text: &str, family: &str) -> Option<Vec<PathBuf>> {
    let mut lines = text.lines();
    if lines.next()? != family {
        return None;
    }
    let files: Vec<PathBuf> = lines.map(PathBuf::from).collect();
    (!files.is_empty()).then_some(files)
}

/// What the list was made for: another weight needs other files.
pub fn key(face: &Face) -> String {
    format!("{} {} {} {} {}", face.family, face.weight, face.bold, face.bold_italic, face.oblique)
}

/// The files that hold the four styles Koral draws.
fn face_files(db: &fontdb::Database, face: &Face) -> Vec<PathBuf> {
    let italic = if face.oblique { fontdb::Style::Oblique } else { fontdb::Style::Italic };
    let wanted = [
        (face.weight, fontdb::Style::Normal),
        (face.bold, fontdb::Style::Normal),
        (face.weight, italic),
        (face.bold_italic, italic),
    ];
    let faces: Vec<_> = db.faces().filter(|info| info.families.iter().any(|(name, _)| *name == face.family)).collect();
    let exact = wanted.iter().all(|style| faces.iter().any(|info| (info.weight.0, info.style) == *style));
    let mut files: Vec<PathBuf> = faces
        .into_iter()
        .filter(|info| !exact || wanted.contains(&(info.weight.0, info.style)))
        .filter_map(|info| match &info.source {
            fontdb::Source::File(path) | fontdb::Source::SharedFile(path, _) => Some(path.clone()),
            fontdb::Source::Binary(_) => None,
        })
        .collect();
    files.sort();
    files.dedup();
    files
}

/// Text sits on a grid measured with `M`: a narrower `i` would leave the cursor behind the text.
fn fixed_width(db: &fontdb::Database, id: fontdb::ID) -> bool {
    let same = db.with_face_data(id, |data, index| {
        let font = ttf_parser::Face::parse(data, index).ok()?;
        let advance = |ch| font.glyph_hor_advance(font.glyph_index(ch)?);
        Some(advance('i')? == advance('M')?)
    });
    same.flatten().unwrap_or(false)
}

/// The families Koral can draw, by the names `Family::Name` finds.
pub fn monospaced(db: &fontdb::Database) -> Vec<String> {
    let mut names: Vec<String> = db
        .faces()
        .filter(|info| info.style == fontdb::Style::Normal && fixed_width(db, info.id))
        .filter_map(|info| Some(info.families.first()?.0.clone()))
        .collect();
    names.sort();
    names.dedup();
    names
}

/// The face to draw with: a font that is missing or proportional would break the grid.
pub fn usable(face: &Face, db: &fontdb::Database) -> Face {
    let installed = db.faces().any(|info| named(info, &face.family) && fixed_width(db, info.id));
    if installed {
        return nearest_weights(face, db);
    }
    let names = monospaced(db);
    let system = db.family_name(&fontdb::Family::Monospace);
    match names.iter().find(|name| *name == system).or(names.first()) {
        Some(family) => nearest_weights(&Face { family: family.clone(), ..face.clone() }, db),
        None => face.clone(),
    }
}

fn named(info: &fontdb::FaceInfo, family: &str) -> bool {
    info.families.iter().any(|(name, _)| name == family)
}

/// Shaping passes over a family that lacks the exact weight, so ask only for weights it has.
fn nearest_weights(face: &Face, db: &fontdb::Database) -> Face {
    let faces: Vec<_> = db.faces().filter(|info| named(info, &face.family)).collect();
    let variable = |info: &&&fontdb::FaceInfo| {
        let parsed = db.with_face_data(info.id, |data, index| Some(ttf_parser::Face::parse(data, index).ok()?.is_variable()));
        parsed.flatten().unwrap_or(false)
    };
    if faces.iter().any(|info| variable(&info)) {
        return face.clone();
    }
    let nearest = |weight: u16, slanted: bool| {
        let styled = faces.iter().filter(|info| (info.style != fontdb::Style::Normal) == slanted);
        styled.map(|info| info.weight.0).min_by_key(|have| have.abs_diff(weight)).unwrap_or(weight)
    };
    Face {
        weight: nearest(face.weight, false),
        bold: nearest(face.bold, false),
        bold_italic: nearest(face.bold_italic, true),
        ..face.clone()
    }
}

/// Fontconfig's scale for a font weight.
fn fontconfig_weight(weight: u16) -> &'static str {
    match weight {
        100 => "0",
        200 => "40",
        300 => "50",
        500 => "100",
        600 => "180",
        700 => "200",
        800 => "205",
        900 => "210",
        _ => "80",
    }
}

/// The files for the four styles, from lines of `file|weight|slant`.
pub fn listed_files(output: &str, face: &Face) -> Vec<PathBuf> {
    let italic = if face.oblique { "110" } else { "100" };
    let wanted = [(face.weight, "0"), (face.bold, "0"), (face.weight, italic), (face.bold_italic, italic)]
        .map(|(weight, slant)| (fontconfig_weight(weight), slant));
    let lines: Vec<(&str, &str, &str)> = output
        .lines()
        .filter_map(|line| {
            let (rest, slant) = line.rsplit_once('|')?;
            let (file, weight) = rest.rsplit_once('|')?;
            Some((file, weight, slant))
        })
        .collect();
    let exact = wanted.iter().all(|style| lines.iter().any(|(_, weight, slant)| (*weight, *slant) == *style));
    let kept = lines.iter().filter(|(_, weight, slant)| !exact || weight.starts_with('[') || wanted.contains(&(*weight, *slant)));
    kept.map(|(file, ..)| PathBuf::from(file)).collect()
}

/// Only the files of the chosen font, so a start does not wait on every font on the disk.
pub fn quick(cache: &Path, face: &Face) -> Option<FontSystem> {
    let text = std::fs::read_to_string(cache).ok()?;
    load(cached_files(&text, &key(face))?, face)
}

/// The same files found through fontconfig, whose cache answers with no list of ours.
pub fn listed(face: &Face) -> Option<FontSystem> {
    let output = Command::new("fc-list")
        .args(["--format=%{file}|%{weight}|%{slant}\\n", "--", &face.family])
        .output()
        .ok()?;
    load(listed_files(&String::from_utf8_lossy(&output.stdout), face), face)
}

fn load(files: Vec<PathBuf>, face: &Face) -> Option<FontSystem> {
    let mut db = fontdb::Database::new();
    for file in files {
        db.load_font_file(file).ok()?;
    }
    let found = db.faces().any(|info| info.families.iter().any(|(name, _)| *name == face.family));
    found.then(|| FontSystem::new_with_locale_and_db(LOCALE.to_string(), db))
}

/// Every font on the system. Reads thousands of file headers, seconds on a cold disk.
pub fn full(cache: Option<&Path>, face: &Face) -> FontSystem {
    let system = FontSystem::new();
    if let Some(cache) = cache
        && let Some(folder) = cache.parent()
        && std::fs::create_dir_all(folder).is_ok()
    {
        let _ = std::fs::write(cache, files_text(&key(face), &face_files(system.db(), face)));
    }
    system
}

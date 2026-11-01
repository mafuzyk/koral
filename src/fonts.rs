use std::path::{Path, PathBuf};

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
    let mut files: Vec<PathBuf> = db
        .faces()
        .filter(|info| info.families.iter().any(|(name, _)| *name == face.family))
        .filter(|info| wanted.contains(&(info.weight.0, info.style)))
        .filter_map(|info| match &info.source {
            fontdb::Source::File(path) | fontdb::Source::SharedFile(path, _) => Some(path.clone()),
            fontdb::Source::Binary(_) => None,
        })
        .collect();
    files.sort();
    files.dedup();
    files
}

/// Only the files of the chosen font, so a start does not wait on every font on the disk.
pub fn quick(cache: &Path, face: &Face) -> Option<FontSystem> {
    let text = std::fs::read_to_string(cache).ok()?;
    let mut db = fontdb::Database::new();
    for file in cached_files(&text, &key(face))? {
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

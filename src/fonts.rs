use std::path::{Path, PathBuf};
use std::process::Command;

use glyphon::{Buffer, FontSystem, Metrics, Shaping, fontdb};

use crate::render::Face;

const LOCALE: &str = "en-US";
/// The line height the renderer draws with, as a multiple of the font size in pixels.
const LINE_HEIGHT: f32 = 1.2;

/// Fast bitmask tracking codepoints `0..1024` that the primary face renders at exact cell width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Coverage {
    bits: [u64; 16],
}

impl Default for Coverage {
    fn default() -> Self {
        let mut bits = [0u64; 16];
        for word in &mut bits[..12] {
            *word = u64::MAX;
        }
        Self { bits }
    }
}

impl Coverage {
    pub fn matches(&self, ch: char) -> bool {
        let cp = ch as u32;
        cp < 1024 && ((self.bits[(cp >> 6) as usize] >> (cp & 63)) & 1) != 0
    }

    pub fn without(mut self, ch: char) -> Self {
        let cp = ch as u32;
        if cp < 1024 {
            self.bits[(cp >> 6) as usize] &= !(1u64 << (cp & 63));
        }
        self
    }

    pub fn from_face(db: &fontdb::Database, face: &Face) -> Self {
        let primary = db
            .faces()
            .filter(|info| named(info, &face.family) && info.style == fontdb::Style::Normal)
            .min_by_key(|info| info.weight.0.abs_diff(face.weight));
        let Some(info) = primary else {
            return Self::default();
        };
        let covered = db.with_face_data(info.id, |data, index| {
            let font = ttf_parser::Face::parse(data, index).ok()?;
            let m_advance = font.glyph_hor_advance(font.glyph_index('M')?)?;
            let mut bits = [0u64; 16];
            for cp in 0u32..1024 {
                let ok = if cp <= 32 {
                    true
                } else if let Some(ch) = char::from_u32(cp) {
                    font.glyph_index(ch)
                        .and_then(|id| font.glyph_hor_advance(id))
                        == Some(m_advance)
                } else {
                    false
                };
                if ok {
                    bits[(cp >> 6) as usize] |= 1u64 << (cp & 63);
                }
            }
            Some(Self { bits })
        });
        covered.flatten().unwrap_or_default()
    }
}

pub fn cache_path() -> Option<PathBuf> {
    let cache = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from);
    Some(
        cache
            .or_else(|| Some(crate::config::home()?.join(".cache")))?
            .join("koral/fonts"),
    )
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
    format!(
        "{} {} {} {} {}",
        face.family, face.weight, face.bold, face.bold_italic, face.oblique
    )
}

/// The files that hold the four styles Koral draws.
fn face_files(db: &fontdb::Database, face: &Face) -> Vec<PathBuf> {
    let italic = if face.oblique {
        fontdb::Style::Oblique
    } else {
        fontdb::Style::Italic
    };
    let wanted = [
        (face.weight, fontdb::Style::Normal),
        (face.bold, fontdb::Style::Normal),
        (face.weight, italic),
        (face.bold_italic, italic),
    ];
    let faces: Vec<_> = db
        .faces()
        .filter(|info| info.families.iter().any(|(name, _)| *name == face.family))
        .collect();
    let exact = wanted.iter().all(|style| {
        faces
            .iter()
            .any(|info| (info.weight.0, info.style) == *style)
    });
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

/// The families that patch a terminal font with icons at the private-use addresses.
fn icon_like(family: &str) -> bool {
    ["Nerd", "Symbols", "Awesome", "Powerline", "Emoji"]
        .iter()
        .any(|mark| family.contains(mark))
}

/// The icon font to draw with, the `Mono` patch first: it shapes every icon to one cell.
fn icon_name(names: &[String]) -> Option<&String> {
    let mono = |name: &&String| icon_like(name.as_str()) && name.contains("Nerd Font Mono");
    names
        .iter()
        .find(mono)
        .or_else(|| names.iter().find(|name| icon_like(name.as_str())))
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
    let installed = db
        .faces()
        .any(|info| named(info, &face.family) && fixed_width(db, info.id));
    if installed {
        return nearest_weights(face, db);
    }
    let names = monospaced(db);
    let system = db.family_name(&fontdb::Family::Monospace);
    // A terminal needs the private-use icons a Nerd Font draws; plain monospace has none.
    // Picking per codepoint is out of scope: the substitute font draws the icons instead.
    let icons = icon_name(&names);
    match icons
        .or_else(|| names.iter().find(|name| *name == system))
        .or(names.first())
    {
        Some(family) => nearest_weights(
            &Face {
                family: family.clone(),
                ..face.clone()
            },
            db,
        ),
        None => face.clone(),
    }
}

fn named(info: &fontdb::FaceInfo, family: &str) -> bool {
    info.families.iter().any(|(name, _)| name == family)
}

/// Shaping passes over a family that lacks the exact weight, so ask only for weights it has.
fn nearest_weights(face: &Face, db: &fontdb::Database) -> Face {
    let faces: Vec<_> = db
        .faces()
        .filter(|info| named(info, &face.family))
        .collect();
    let variable = |info: &&&fontdb::FaceInfo| {
        let parsed = db.with_face_data(info.id, |data, index| {
            Some(ttf_parser::Face::parse(data, index).ok()?.is_variable())
        });
        parsed.flatten().unwrap_or(false)
    };
    if faces.iter().any(|info| variable(&info)) {
        return face.clone();
    }
    let nearest = |weight: u16, slanted: bool| {
        let styled = faces
            .iter()
            .filter(|info| (info.style != fontdb::Style::Normal) == slanted);
        styled
            .map(|info| info.weight.0)
            .min_by_key(|have| have.abs_diff(weight))
            .unwrap_or(weight)
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
    let wanted = [
        (face.weight, "0"),
        (face.bold, "0"),
        (face.weight, italic),
        (face.bold_italic, italic),
    ]
    .map(|(weight, slant)| (fontconfig_weight(weight), slant));
    let lines: Vec<(&str, &str, &str)> = output
        .lines()
        .filter_map(|line| {
            let (rest, slant) = line.rsplit_once('|')?;
            let (file, weight) = rest.rsplit_once('|')?;
            Some((file, weight, slant))
        })
        .collect();
    let exact = wanted.iter().all(|style| {
        lines
            .iter()
            .any(|(_, weight, slant)| (*weight, *slant) == *style)
    });
    let kept = lines.iter().filter(|(_, weight, slant)| {
        !exact || weight.starts_with('[') || wanted.contains(&(*weight, *slant))
    });
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
    load(
        listed_files(&String::from_utf8_lossy(&output.stdout), face),
        face,
    )
}

/// A small initial font database for the highest-priority icon fallback.
/// If fontconfig cannot identify and load that family, keep the full startup path.
pub fn listed_fallback(face: &Face) -> Option<FontSystem> {
    let output = Command::new("fc-list")
        .arg("--format=%{family[0]}|%{file}\n")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let output = std::str::from_utf8(&output.stdout).ok()?;
    let entries: Vec<_> = output
        .lines()
        .filter_map(|line| line.split_once('|'))
        .collect();
    let family = entries
        .iter()
        .map(|(family, _)| *family)
        .filter(|family| family.contains("Nerd Font Mono"))
        .min()?;
    let mut files: Vec<_> = entries
        .iter()
        .filter(|(name, _)| *name == family)
        .map(|(_, file)| PathBuf::from(file))
        .collect();
    files.sort_unstable();
    files.dedup();
    let candidate = Face {
        family: family.to_owned(),
        ..face.clone()
    };
    let fonts = load(files, &candidate)?;
    monospaced(fonts.db())
        .iter()
        .any(|name| name == family)
        .then_some(fonts)
}

fn load(files: Vec<PathBuf>, face: &Face) -> Option<FontSystem> {
    let mut db = fontdb::Database::new();
    for file in files {
        db.load_font_file(file).ok()?;
    }
    let found = db
        .faces()
        .any(|info| info.families.iter().any(|(name, _)| *name == face.family));
    found.then(|| FontSystem::new_with_locale_and_db(LOCALE.to_string(), db))
}

/// Every glyph a screen is likely to ask for, so the first frame that shows a fallback face does not
/// pay for reading it. One character per block, each picked by asking the db which face draws it:
/// Latin accents, arrows and box drawing (primary), geometric and dingbats, math operators (primary),
/// math alphanumerics `𝐀` (Noto Sans Math), misc technical `⏵` (Noto Sans Symbols 2), supplemental
/// arrows `⤈` (Math), misc symbols `⚙` (Noto Sans Symbols), geometric extended `🞀` (Math), braille
/// `⣿` and the Nerd Font private use `\u{e0b0}` (primary), and emoji.
pub const WARM_SAMPLE: &str = "Revisão Migração ação ← → ↑ ↓ ─ │ ┌ ┐ ◆ ◇ ● ○ ✓ ✗ • … ≈ ≠ ≤ ± ∑ 𝐀 ⏵ ✦ ⤈ ⚙ 🞀 ⣿ 😀 🚀 ❤ \u{e0b0}";

/// The families that shaped these glyphs, in the order they appear: the log names them.
pub fn families(font_system: &FontSystem, buffer: &Buffer) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for run in buffer.layout_runs() {
        for glyph in run.glyphs {
            let family = font_system
                .db()
                .face(glyph.font_id)
                .and_then(|info| info.families.first().map(|(name, _)| name.clone()));
            if let Some(family) = family
                && !found.contains(&family)
            {
                found.push(family);
            }
        }
    }
    found
}

/// Shapes the sample once, so the faces it reaches are loaded before a frame needs them.
// shortcut: no CJK on purpose. Its face is a 32 MB ttc that a screen without CJK never needs, and a
// screen that does need it pays 0.18 ms on the first such line, measured. Do not add 漢 here.
pub fn warm(font_system: &mut FontSystem, face: &Face, points: f32) {
    let logged = crate::lag::enabled();
    let start = std::time::Instant::now();
    let pixels = points * 96.0 / 72.0;
    let metrics = Metrics::new(pixels, (pixels * LINE_HEIGHT).round());
    let attrs = crate::render::attrs(face, false, false);
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_text(WARM_SAMPLE, &attrs, Shaping::Advanced, None);
    buffer.shape_until_scroll(font_system, false);
    if logged {
        let touched = families(font_system, &buffer);
        crate::lag::warmed(crate::lag::millis(start), &touched, WARM_SAMPLE);
    }
}

/// Every font on the system. Reads thousands of file headers, seconds on a cold disk.
pub fn full(cache: Option<&Path>, face: &Face) -> FontSystem {
    let system = FontSystem::new();
    if let Some(cache) = cache
        && let Some(folder) = cache.parent()
        && std::fs::create_dir_all(folder).is_ok()
    {
        let _ = std::fs::write(
            cache,
            files_text(&key(face), &face_files(system.db(), face)),
        );
    }
    system
}

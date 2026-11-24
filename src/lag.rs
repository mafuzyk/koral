use std::io::Write;
use std::sync::OnceLock;
use std::time::Instant;

pub fn startup(stage: &str) {
    static START: OnceLock<Option<Instant>> = OnceLock::new();
    if let Some(start) =
        START.get_or_init(|| std::env::var_os("KORAL_STARTUP_LOG").map(|_| Instant::now()))
    {
        eprintln!("startup: {stage} {}", start.elapsed().as_micros());
    }
}

/// A frame past this is a stutter worth a line: a 60 Hz refresh gives 16 ms.
const SLOW: f32 = 8.0;

/// The most codepoints the log prints for the slowest line.
pub const CODES: usize = 8;

/// What the slowest shaped line of a frame had, so the log says what made it slow.
#[derive(Debug, Default, Clone)]
pub struct LineCost {
    pub micros: u64,
    pub chars: usize,
    pub spans: usize,
    pub colors: usize,
    pub non_ascii: bool,
    /// The codepoints it holds, as hex, at most `CODES` of them.
    pub codes: String,
    /// The families that shaped it, comma separated.
    pub face: String,
    /// Whether any glyph came from a face other than the one the config asks for.
    pub fallback: bool,
    /// The same line shaped a second time in the same frame: what a cold face costs against what
    /// every shape of that line costs.
    pub again: u64,
}

/// Where one frame's time went, in the split that says which part to fix.
#[derive(Debug, Default, Clone)]
pub struct Cost {
    /// The whole draw, from the grid walk to the present.
    pub total: f32,
    /// Walking the grid and turning the visible cells into quads and spans.
    pub grid: f32,
    /// The wait inside `grid` for a session lock the reader thread was holding.
    pub wait: f32,
    /// Shaping the lines the cache missed, and preparing the text.
    pub text: f32,
    /// The shaping inside `text`: ours, one `Buffer` per unseen line.
    pub shape: f32,
    /// The `glyphon` prepare inside `text`.
    pub prepare: f32,
    /// Picture textures, their uploads and the layer order.
    pub pictures: f32,
    /// Encoding the vertices, submitting and presenting.
    pub present: f32,
    /// The wait for the next surface image: time the window server took, not work.
    pub acquire: f32,
    pub lines: usize,
    pub shaped: usize,
    /// Glyphs the drawn lines carry, which is what the text prepare walks.
    pub glyphs: usize,
    /// Of `shaped`, the lines no earlier frame had shaped.
    pub fresh: usize,
    /// Of `shaped`, the lines shaped again after an eviction.
    pub again: usize,
    /// Lines the eviction dropped in this frame.
    pub evicted: usize,
    pub uploads: usize,
    /// The slowest line shaped in this frame.
    pub worst: LineCost,
}

/// Milliseconds, the unit the log reads in.
pub fn millis(from: Instant) -> f32 {
    from.elapsed().as_secs_f32() * 1000.0
}

/// Whether the frame log is on, so the work that only describes a frame stays off when it is not.
pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("KORAL_FRAME_LOG").is_some())
}

fn log() -> Option<&'static std::fs::File> {
    static LOG: OnceLock<Option<std::fs::File>> = OnceLock::new();
    LOG.get_or_init(|| {
        let path = std::env::var_os("KORAL_FRAME_LOG")?;
        std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
            .ok()
    })
    .as_ref()
}

/// `KORAL_FRAME_LOG=<path>` appends one line per slow frame, so a stutter is
/// explained by numbers instead of a guess.
pub fn note(cost: &Cost) {
    if cost.total < SLOW {
        return;
    }
    let Some(mut file) = log() else { return };
    let worst = &cost.worst;
    let _ = writeln!(
        file,
        "frame {:.1}ms grid={:.1} wait={:.1} text={:.1} shape={:.1} prepare={:.1} pictures={:.1} present={:.1} acquire={:.1} \
         lines={} shaped={} glyphs={} fresh={} again={} evicted={} uploads={} worst={}us chars={} spans={} colors={} nonascii={} again_worst={}us \
         face={} fallback={} codes={}",
        cost.total,
        cost.grid,
        cost.wait,
        cost.text,
        cost.shape,
        cost.prepare,
        cost.pictures,
        cost.present,
        cost.acquire,
        cost.lines,
        cost.shaped,
        cost.glyphs,
        cost.fresh,
        cost.again,
        cost.evicted,
        cost.uploads,
        worst.micros,
        worst.chars,
        worst.spans,
        worst.colors,
        u8::from(worst.non_ascii),
        worst.again,
        if worst.face.is_empty() {
            "?"
        } else {
            worst.face.as_str()
        },
        u8::from(worst.fallback),
        worst.codes
    );
}

/// The warm-up runs once, off the frame: one line says it happened, what it cost and what it touched.
pub fn warmed(millis: f32, faces: &[String], sample: &str) {
    let Some(mut file) = log() else { return };
    let _ = writeln!(
        file,
        "warm {millis:.1}ms faces={} sample=\"{sample}\"",
        faces.join(",")
    );
}

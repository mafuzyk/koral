use std::io::Write;
use std::sync::OnceLock;
use std::time::Instant;

/// A frame past this is a stutter worth a line: a 60 Hz refresh gives 16 ms.
const SLOW: f32 = 8.0;

/// Where one frame's time went, in the split that says which part to fix.
#[derive(Debug, Default, Clone, Copy)]
pub struct Cost {
    /// The whole draw, from the grid walk to the present.
    pub total: f32,
    /// Walking the grid and turning the visible cells into quads and spans.
    pub grid: f32,
    /// The wait inside `grid` for a session lock the reader thread was holding.
    pub wait: f32,
    /// Shaping the lines the cache missed, and preparing the text.
    pub text: f32,
    /// Picture textures, their uploads and the layer order.
    pub pictures: f32,
    /// Encoding the vertices, submitting and presenting.
    pub present: f32,
    /// The wait for the next surface image: time the window server took, not work.
    pub acquire: f32,
    pub lines: usize,
    pub shaped: usize,
    pub uploads: usize,
}

/// Milliseconds, the unit the log reads in.
pub fn millis(from: Instant) -> f32 {
    from.elapsed().as_secs_f32() * 1000.0
}

/// `KORAL_FRAME_LOG=<path>` appends one line per slow frame, so a stutter is
/// explained by numbers instead of a guess.
pub fn note(cost: &Cost) {
    if cost.total < SLOW {
        return;
    }
    static LOG: OnceLock<Option<std::fs::File>> = OnceLock::new();
    let Some(file) = LOG.get_or_init(|| {
        let path = std::env::var_os("KORAL_FRAME_LOG")?;
        std::fs::OpenOptions::new().append(true).create(true).open(path).ok()
    }) else {
        return;
    };
    let mut file = file;
    let _ = writeln!(
        file,
        "frame {:.1}ms grid={:.1} wait={:.1} text={:.1} pictures={:.1} present={:.1} acquire={:.1} lines={} shaped={} uploads={}",
        cost.total,
        cost.grid,
        cost.wait,
        cost.text,
        cost.pictures,
        cost.present,
        cost.acquire,
        cost.lines,
        cost.shaped,
        cost.uploads
    );
}

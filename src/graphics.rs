use std::collections::{HashMap, VecDeque};
use std::io::{Cursor, Read};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use base64::Engine;
use base64::engine::general_purpose::STANDARD_NO_PAD;

// The stream is untrusted: limits hold for the image, the store and the transfer.
pub const MAX_IMAGE: usize = 64 * 1024 * 1024;
const MAX_STORE: usize = 320 * 1024 * 1024;
const MAX_TRANSFER: usize = 128 * 1024 * 1024;
const MAX_CELLS: u32 = 1024;
const ANCHOR_RANGE: u32 = 0x7FFE;
const FIRST_AUTOMATIC_ID: u32 = 0x8000_0000;

static GENERATION: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request {
    pub image: u32,
    pub number: u32,
    pub columns: u32,
    pub rows: u32,
    pub keep_cursor: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub reply: Option<String>,
    pub place: Option<Request>,
}

pub struct Image {
    pub generation: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub image: u32,
    pub number: u32,
    pub columns: u32,
    pub rows: u32,
}

#[derive(Clone, Copy)]
struct Keys {
    action: u8,
    format: u32,
    medium: u8,
    compressed: bool,
    width: u32,
    height: u32,
    image: u32,
    number: u32,
    columns: u32,
    rows: u32,
    quiet: u32,
    keep_cursor: bool,
    delete: u8,
    more: bool,
}

impl Default for Keys {
    fn default() -> Self {
        Self {
            action: b't',
            format: 32,
            medium: b'd',
            compressed: false,
            width: 0,
            height: 0,
            image: 0,
            number: 0,
            columns: 0,
            rows: 0,
            quiet: 0,
            keep_cursor: false,
            delete: b'a',
            more: false,
        }
    }
}

struct Transfer {
    keys: Keys,
    data: Vec<u8>,
    too_big: bool,
}

struct Decoded {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

/// The kitty graphics protocol for one terminal, without the grid or the GPU.
// shortcut: no cropping, z-index, animation or unicode placeholders, add them when a program needs one.
pub struct Graphics {
    images: HashMap<u32, Arc<Image>>,
    order: VecDeque<u32>,
    used: usize,
    placements: HashMap<u32, Placement>,
    transfer: Option<Transfer>,
    next_anchor: u32,
    next_id: u32,
    cell: (u32, u32),
}

impl Default for Graphics {
    fn default() -> Self {
        Self {
            images: HashMap::new(),
            order: VecDeque::new(),
            used: 0,
            placements: HashMap::new(),
            transfer: None,
            next_anchor: 0,
            next_id: FIRST_AUTOMATIC_ID,
            cell: (1, 1),
        }
    }
}

impl Graphics {
    pub fn set_cell(&mut self, width: u32, height: u32) {
        self.cell = (width.max(1), height.max(1));
    }

    pub fn handle(&mut self, body: &[u8]) -> Outcome {
        let Some((sent, payload)) = parse(body) else {
            return Outcome::default();
        };
        let mut transfer = self.transfer.take().unwrap_or(Transfer { keys: sent, data: Vec::new(), too_big: false });

        let payload = payload.strip_suffix(b"==").or(payload.strip_suffix(b"=")).unwrap_or(payload);
        if transfer.data.len() + payload.len() > MAX_TRANSFER {
            transfer.too_big = true;
            transfer.data = Vec::new();
        }
        if !transfer.too_big {
            transfer.data.extend_from_slice(payload);
        }
        if sent.more {
            self.transfer = Some(transfer);
            return Outcome::default();
        }
        self.finish(&transfer)
    }

    fn finish(&mut self, transfer: &Transfer) -> Outcome {
        let keys = transfer.keys;
        match keys.action {
            b'd' => {
                self.delete(keys);
                Outcome::default()
            }
            b'p' => match self.request(keys, keys.image) {
                Some(request) => Outcome { reply: reply(keys, Ok(())), place: Some(request) },
                None => Outcome { reply: reply(keys, Err("ENOENT:no such image")), place: None },
            },
            b'q' => Outcome { reply: reply(keys, decode(transfer).map(|_| ())), place: None },
            b't' | b'T' => match decode(transfer) {
                Ok(decoded) => {
                    let id = self.store(keys.image, decoded);
                    let place = if keys.action == b'T' { self.request(keys, id) } else { None };
                    Outcome { reply: reply(keys, Ok(())), place }
                }
                Err(error) => Outcome { reply: reply(keys, Err(error)), place: None },
            },
            _ => Outcome::default(),
        }
    }

    fn store(&mut self, id: u32, decoded: Decoded) -> u32 {
        let id = if id == 0 {
            self.next_id = self.next_id.wrapping_add(1) | FIRST_AUTOMATIC_ID;
            self.next_id
        } else {
            id
        };
        self.forget(id);

        self.used += decoded.rgba.len();
        self.order.push_back(id);
        let generation = GENERATION.fetch_add(1, Ordering::Relaxed);
        let Decoded { width, height, rgba } = decoded;
        self.images.insert(id, Arc::new(Image { generation, width, height, rgba }));

        while self.used > MAX_STORE
            && let Some(oldest) = self.order.front().copied()
        {
            self.forget(oldest);
            self.placements.retain(|_, placement| placement.image != oldest);
        }
        id
    }

    fn forget(&mut self, id: u32) {
        if let Some(image) = self.images.remove(&id) {
            self.used -= image.rgba.len();
            self.order.retain(|other| *other != id);
        }
    }

    fn request(&self, keys: Keys, id: u32) -> Option<Request> {
        let image = self.images.get(&id)?;
        let (cell_width, cell_height) = (u64::from(self.cell.0), u64::from(self.cell.1));
        let (width, height) = (u64::from(image.width), u64::from(image.height));
        let (columns, rows) = (u64::from(keys.columns), u64::from(keys.rows));

        let fitted_columns = match (columns, rows) {
            (0, 0) => width.div_ceil(cell_width),
            (0, rows) => (rows * cell_height * width).div_ceil(height * cell_width),
            (columns, _) => columns,
        };
        let fitted_rows = match (columns, rows) {
            (0, 0) => height.div_ceil(cell_height),
            (columns, 0) => (columns * cell_width * height).div_ceil(width * cell_height),
            (_, rows) => rows,
        };
        let clamp = |cells: u64| cells.clamp(1, u64::from(MAX_CELLS)) as u32;
        Some(Request {
            image: id,
            number: keys.number,
            columns: clamp(fitted_columns),
            rows: clamp(fitted_rows),
            keep_cursor: keys.keep_cursor,
        })
    }

    /// The anchor for a placement. `here` holds the anchors already on the target
    /// cell: a video repainting one spot must not pile anchors onto it.
    pub fn place(&mut self, request: Request, here: &[u32]) -> u32 {
        let Request { image, number, columns, rows, .. } = request;
        let same = |placement: &Placement| placement.image == image && placement.number == number;
        let placement = Placement { image, number, columns, rows };

        // mpv sends each frame with no id, so nothing else ties a frame to the one before it.
        let frame = |id: u32| id >= FIRST_AUTOMATIC_ID;
        let replaces = |placement: &Placement| {
            let (old, size) = (placement.image, (placement.columns, placement.rows));
            same(placement) || (frame(image) && frame(old) && number == placement.number && size == (columns, rows))
        };
        if let Some(anchor) = here.iter().find(|anchor| self.placements.get(anchor).is_some_and(replaces)).copied() {
            let old = self.placements.insert(anchor, placement).map_or(image, |placement| placement.image);
            if old != image && frame(old) && !self.placements.values().any(|placement| placement.image == old) {
                self.forget(old);
            }
            return anchor;
        }
        if number != 0 {
            self.placements.retain(|_, placement| !same(placement));
        }
        let anchor = self.next_anchor;
        self.next_anchor = (anchor + 1) % ANCHOR_RANGE;
        self.placements.insert(anchor, placement);
        anchor
    }

    pub fn placement(&self, anchor: u32) -> Option<&Placement> {
        self.placements.get(&anchor)
    }

    pub fn image(&self, id: u32) -> Option<&Arc<Image>> {
        self.images.get(&id)
    }

    /// Rows above the viewport that can still reach into it.
    pub fn tallest(&self) -> usize {
        self.placements.values().map(|placement| placement.rows as usize).max().unwrap_or(0)
    }

    fn delete(&mut self, keys: Keys) {
        match keys.delete {
            b'a' => self.placements.clear(),
            b'A' => {
                self.placements.clear();
                self.images.clear();
                self.order.clear();
                self.used = 0;
            }
            b'i' | b'I' => {
                self.placements.retain(|_, placement| placement.image != keys.image);
                if keys.delete == b'I' {
                    self.forget(keys.image);
                }
            }
            _ => {}
        }
    }
}

fn parse(body: &[u8]) -> Option<(Keys, &[u8])> {
    let body = body.strip_prefix(b"G")?;
    let split = body.iter().position(|byte| *byte == b';').unwrap_or(body.len());
    let (control, payload) = body.split_at(split);

    let mut keys = Keys::default();
    for pair in std::str::from_utf8(control).ok()?.split(',') {
        let (key, value) = pair.split_once('=')?;
        let number = value.parse().unwrap_or(0);
        let letter = value.bytes().next().unwrap_or(0);
        match key {
            "a" => keys.action = letter,
            "f" => keys.format = number,
            "t" => keys.medium = letter,
            "o" => keys.compressed = letter == b'z',
            "s" => keys.width = number,
            "v" => keys.height = number,
            "i" => keys.image = number,
            "p" => keys.number = number,
            "c" => keys.columns = number,
            "r" => keys.rows = number,
            "q" => keys.quiet = number,
            "C" => keys.keep_cursor = number == 1,
            "d" => keys.delete = letter,
            "m" => keys.more = number == 1,
            _ => {}
        }
    }
    Some((keys, payload.get(1..).unwrap_or(&[])))
}

fn reply(keys: Keys, result: Result<(), &str>) -> Option<String> {
    let silent = match result {
        Ok(()) => keys.quiet >= 1,
        Err(_) => keys.quiet >= 2,
    };
    if silent || keys.image == 0 {
        return None;
    }
    Some(format!("\x1b_Gi={};{}\x1b\\", keys.image, result.err().unwrap_or("OK")))
}

fn decode(transfer: &Transfer) -> Result<Decoded, &'static str> {
    let keys = transfer.keys;
    // Reading paths or shared memory would let any printed file read this machine.
    if keys.medium != b'd' {
        return Err("ENOTSUPPORTED:only direct transmission");
    }
    if transfer.too_big {
        return Err("EFBIG:image too large");
    }
    let raw = STANDARD_NO_PAD.decode(&transfer.data).map_err(|_| "EINVAL:bad base64")?;
    let raw = if keys.compressed { inflate(&raw)? } else { raw };
    match keys.format {
        24 | 32 => pixels(keys, &raw),
        100 => decode_png(&raw),
        _ => Err("EINVAL:unknown format"),
    }
}

fn inflate(raw: &[u8]) -> Result<Vec<u8>, &'static str> {
    let mut inflated = Vec::new();
    flate2::read::ZlibDecoder::new(raw)
        .take(MAX_IMAGE as u64 + 1)
        .read_to_end(&mut inflated)
        .map_err(|_| "EINVAL:bad zlib data")?;
    if inflated.len() > MAX_IMAGE {
        return Err("EFBIG:image too large");
    }
    Ok(inflated)
}

fn pixels(keys: Keys, raw: &[u8]) -> Result<Decoded, &'static str> {
    let (width, height) = (keys.width, keys.height);
    let count = u64::from(width) * u64::from(height);
    if count == 0 {
        return Err("EINVAL:missing size");
    }
    if count * 4 > MAX_IMAGE as u64 {
        return Err("EFBIG:image too large");
    }
    let stride = (keys.format / 8) as usize;
    if raw.len() != count as usize * stride {
        return Err("EINVAL:wrong data size");
    }
    let rgba = if stride == 4 {
        raw.to_vec()
    } else {
        raw.as_chunks::<3>().0.iter().flat_map(|[r, g, b]| [*r, *g, *b, 255]).collect()
    };
    Ok(Decoded { width, height, rgba })
}

fn decode_png(raw: &[u8]) -> Result<Decoded, &'static str> {
    let limits = png::Limits { bytes: MAX_IMAGE };
    let mut decoder = png::Decoder::new_with_limits(Cursor::new(raw), limits);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|_| "EINVAL:bad png")?;
    let mut buffer = vec![0; reader.output_buffer_size().ok_or("EFBIG:image too large")?];
    let info = reader.next_frame(&mut buffer).map_err(|_| "EINVAL:bad png")?;
    let (width, height) = (info.width, info.height);
    if u64::from(width) * u64::from(height) * 4 > MAX_IMAGE as u64 {
        return Err("EFBIG:image too large");
    }

    let data = &buffer[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data.as_chunks::<3>().0.iter().flat_map(|[r, g, b]| [*r, *g, *b, 255]).collect(),
        png::ColorType::GrayscaleAlpha => data.as_chunks::<2>().0.iter().flat_map(|[v, a]| [*v, *v, *v, *a]).collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|gray| [*gray, *gray, *gray, 255]).collect(),
        png::ColorType::Indexed => return Err("EINVAL:bad png"),
    };
    Ok(Decoded { width, height, rgba })
}

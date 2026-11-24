use std::collections::{HashMap, VecDeque};
use std::io::{Cursor, Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use base64::Engine;
use base64::engine::general_purpose::STANDARD_NO_PAD;

// The stream is untrusted: limits hold for the image, the store and the transfer.
pub const MAX_IMAGE: usize = 64 * 1024 * 1024;
const MAX_STORE: usize = 320 * 1024 * 1024;
// A decode holds the output buffer and the RGBA copy at once, and neither may outgrow the store.
const _: () = assert!(2 * MAX_IMAGE <= MAX_STORE);
pub const MAX_TRANSFER: usize = 128 * 1024 * 1024;
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
    /// The part of the image to show: x, y, width and height in pixels.
    pub crop: [u32; 4],
    /// The stacking order: below zero is under the text.
    pub z: i32,
}

/// Where a delete command points, in screen cells counting from the top left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    /// The cell the cursor is on.
    Cursor,
    /// One cell, and the z-index it must have when the client named one.
    Cell {
        column: u32,
        row: u32,
        z: Option<i32>,
    },
    Column(u32),
    Row(u32),
    Z(i32),
}

/// A delete command the grid has to resolve: the cells live in the terminal, not here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Delete {
    pub area: Area,
    pub free: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub reply: Option<String>,
    pub place: Option<Request>,
    pub delete: Option<Delete>,
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
    pub crop: [u32; 4],
    pub z: i32,
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
    crop: [u32; 4],
    delete: u8,
    more: bool,
    z: i32,
    virtual_placement: bool,
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
            crop: [0; 4],
            delete: b'a',
            more: false,
            z: 0,
            virtual_placement: false,
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
// shortcut: no animation support; unicode placeholders and z-index are supported.
pub struct Graphics {
    images: HashMap<u32, Arc<Image>>,
    order: VecDeque<u32>,
    used: usize,
    placements: HashMap<u32, Placement>,
    virtuals: HashMap<(u32, u32), Placement>,
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
            virtuals: HashMap::new(),
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
        if let Some(rest) = body.strip_prefix(b"G") {
            let split = rest.iter().position(|byte| *byte == b';');
            let (control, payload) = split.map_or((rest, &rest[rest.len()..]), |at| {
                (&rest[..at], &rest[at + 1..])
            });
            trace(control, payload.len());
        }
        let Some((sent, payload)) = parse(body) else {
            return Outcome::default();
        };
        let mut transfer = self.transfer.take().unwrap_or(Transfer {
            keys: sent,
            data: Vec::new(),
            too_big: false,
        });

        let payload = payload
            .strip_suffix(b"==")
            .or(payload.strip_suffix(b"="))
            .unwrap_or(payload);
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
        // Reading paths or shared memory would let any printed file read this machine, so a medium
        // that carries data must be answered with an error instead of being dropped.
        if matches!(keys.action, b't' | b'T' | b'q') && keys.medium != b'd' {
            return Outcome {
                reply: reply(keys, Err("ENOTSUPPORTED:only direct transmission")),
                place: None,
                delete: None,
            };
        }
        match keys.action {
            b'd' => self.delete(keys),
            b'p' => match self.request(keys, keys.image) {
                Some(request) if keys.virtual_placement => {
                    self.place_virtual(request);
                    Outcome {
                        reply: reply(keys, Ok(())),
                        place: None,
                        delete: None,
                    }
                }
                Some(request) => Outcome {
                    reply: reply(keys, Ok(())),
                    place: Some(request),
                    delete: None,
                },
                None => Outcome {
                    reply: reply(keys, Err("ENOENT:no such image")),
                    place: None,
                    delete: None,
                },
            },
            b'q' => Outcome {
                reply: reply(keys, decode(transfer).map(|_| ())),
                place: None,
                delete: None,
            },
            b't' | b'T' => match decode(transfer) {
                Ok(decoded) => {
                    let id = self.store(keys.image, decoded);
                    let place = if keys.action == b'T' {
                        let request = self.request(keys, id);
                        if keys.virtual_placement {
                            if let Some(request) = request {
                                self.place_virtual(request);
                            }
                            None
                        } else {
                            request
                        }
                    } else {
                        None
                    };
                    Outcome {
                        reply: reply(keys, Ok(())),
                        place,
                        delete: None,
                    }
                }
                Err(error) => Outcome {
                    reply: reply(keys, Err(error)),
                    place: None,
                    delete: None,
                },
            },
            _ => Outcome {
                reply: reply(keys, Err("EINVAL:unknown action")),
                place: None,
                delete: None,
            },
        }
    }

    fn store(&mut self, id: u32, decoded: Decoded) -> u32 {
        let id = if id == 0 {
            self.next_id = self.next_id.wrapping_add(1) | FIRST_AUTOMATIC_ID;
            self.next_id
        } else {
            id
        };
        // A repaint of the same size keeps its placements: video frames and clients that refresh
        // one spot rely on the anchor staying on its cell. A different size makes the stored cell
        // rectangle meaningless, so the placements that used it go.
        let was = self
            .images
            .get(&id)
            .map(|image| (image.width, image.height));
        self.forget(id);
        if was.is_some_and(|size| size != (decoded.width, decoded.height)) {
            self.placements.retain(|_, placement| placement.image != id);
            self.virtuals.retain(|_, placement| placement.image != id);
        }

        self.used += decoded.rgba.len();
        self.order.push_back(id);
        let generation = GENERATION.fetch_add(1, Ordering::Relaxed);
        let Decoded {
            width,
            height,
            rgba,
        } = decoded;
        self.images.insert(
            id,
            Arc::new(Image {
                generation,
                width,
                height,
                rgba,
            }),
        );

        while self.used > MAX_STORE {
            // A preview nothing shows goes before an image a placement still keeps on screen; only
            // when every image has one does the oldest transmission go.
            let placed = |id: &u32| {
                self.placements
                    .values()
                    .any(|placement| placement.image == *id)
                    || self
                        .virtuals
                        .values()
                        .any(|placement| placement.image == *id)
            };
            let victim = self
                .order
                .iter()
                .find(|id| !placed(id))
                .copied()
                .or_else(|| self.order.front().copied());
            let Some(victim) = victim else { break };
            self.forget(victim);
            self.placements
                .retain(|_, placement| placement.image != victim);
            self.virtuals
                .retain(|_, placement| placement.image != victim);
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
        let [x, y, width, height] = keys.crop;
        let (x, y) = (x.min(image.width - 1), y.min(image.height - 1));
        let part = |size: u32, left: u32| if size == 0 { left } else { size.min(left) };
        let crop = [
            x,
            y,
            part(width, image.width - x),
            part(height, image.height - y),
        ];
        let (width, height) = (u64::from(crop[2]), u64::from(crop[3]));
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
            crop,
            z: keys.z,
        })
    }

    /// The anchor for a placement. `here` holds the anchors already on the target
    /// cell: a video repainting one spot must not pile anchors onto it. A rectangle with
    /// no cells is refused: it would hold a cell and draw nothing.
    pub fn place(&mut self, request: Request, here: &[u32]) -> Option<u32> {
        let Request {
            image,
            number,
            columns,
            rows,
            crop,
            z,
            ..
        } = request;
        if columns == 0 || rows == 0 {
            return None;
        }
        let same = |placement: &Placement| placement.image == image && placement.number == number;
        let placement = Placement {
            image,
            number,
            columns,
            rows,
            crop,
            z,
        };

        // mpv sends each frame with no id, so nothing else ties a frame to the one before it.
        let frame = |id: u32| id >= FIRST_AUTOMATIC_ID;
        let replaces = |placement: &Placement| {
            let (old, size) = (placement.image, (placement.columns, placement.rows));
            same(placement)
                || (frame(image)
                    && frame(old)
                    && number == placement.number
                    && size == (columns, rows))
        };
        if let Some(anchor) = here
            .iter()
            .find(|anchor| self.placements.get(anchor).is_some_and(replaces))
            .copied()
        {
            let old = self
                .placements
                .insert(anchor, placement)
                .map_or(image, |placement| placement.image);
            if old != image
                && frame(old)
                && !self
                    .placements
                    .values()
                    .any(|placement| placement.image == old)
            {
                self.forget(old);
            }
            return Some(anchor);
        }
        if number != 0 {
            self.placements.retain(|_, placement| !same(placement));
        }
        let anchor = self.next_anchor;
        self.next_anchor = (anchor + 1) % ANCHOR_RANGE;
        self.placements.insert(anchor, placement);
        Some(anchor)
    }

    fn place_virtual(&mut self, request: Request) {
        if request.columns == 0 || request.rows == 0 {
            return;
        }
        let placement = Placement {
            image: request.image,
            number: request.number,
            columns: request.columns,
            rows: request.rows,
            crop: request.crop,
            z: request.z,
        };
        self.virtuals
            .insert((request.image, request.number), placement);
    }

    pub fn virtual_placement(&self, image: u32, number: u32) -> Option<&Placement> {
        self.virtuals.get(&(image, number)).or_else(|| {
            (number == 0)
                .then(|| {
                    self.virtuals
                        .iter()
                        .find(|((img, _), _)| *img == image)
                        .map(|(_, p)| p)
                })
                .flatten()
        })
    }

    pub fn placement(&self, anchor: u32) -> Option<&Placement> {
        self.placements.get(&anchor)
    }

    pub fn has_placements(&self) -> bool {
        !self.placements.is_empty()
    }

    pub fn retain_anchors(&mut self, mut keep: impl FnMut(u32) -> bool) {
        self.placements.retain(|anchor, _| keep(*anchor));
    }

    /// Drops every placement and keeps the data, for a screen that goes away under the images.
    pub fn clear_placements(&mut self) {
        self.placements.clear();
        self.virtuals.clear();
    }

    pub fn image(&self, id: u32) -> Option<&Arc<Image>> {
        self.images.get(&id)
    }

    /// Bytes the stored images hold.
    pub fn used(&self) -> usize {
        self.used
    }

    /// Rows above the viewport that can still reach into it.
    pub fn tallest(&self) -> usize {
        self.placements
            .values()
            .chain(self.virtuals.values())
            .map(|placement| placement.rows as usize)
            .max()
            .unwrap_or(0)
    }

    /// `d` names what to delete. The modes that name cells go back to the grid; the rest are
    /// settled here. Answering instead of staying silent is what lets a client fall back.
    fn delete(&mut self, keys: Keys) -> Outcome {
        let free = keys.delete.is_ascii_uppercase();
        let chosen = |placement: &Placement| keys.number == 0 || placement.number == keys.number;
        // The client counts cells from one.
        let cell = |number: u32| number.saturating_sub(1);
        let (x, y) = (keys.crop[0], keys.crop[1]);
        let area = match keys.delete.to_ascii_lowercase() {
            b'a' => {
                self.placements.clear();
                self.virtuals.clear();
                if free {
                    self.images.clear();
                    self.order.clear();
                    self.used = 0;
                }
                return Outcome::default();
            }
            b'i' => {
                self.placements
                    .retain(|_, placement| placement.image != keys.image || !chosen(placement));
                self.virtuals
                    .retain(|_, placement| placement.image != keys.image || !chosen(placement));
                self.release(keys.image, free);
                return Outcome::default();
            }
            b'r' => {
                self.placements
                    .retain(|_, placement| !(x..=y).contains(&placement.image));
                self.virtuals
                    .retain(|_, placement| !(x..=y).contains(&placement.image));
                for id in x..=y {
                    self.release(id, free);
                }
                return Outcome::default();
            }
            b'c' => Area::Cursor,
            b'p' => Area::Cell {
                column: cell(x),
                row: cell(y),
                z: None,
            },
            b'q' => Area::Cell {
                column: cell(x),
                row: cell(y),
                z: Some(keys.z),
            },
            b'x' => Area::Column(cell(x)),
            b'y' => Area::Row(cell(y)),
            b'z' => {
                let mut removed = Vec::new();
                self.virtuals.retain(|_, placement| {
                    if placement.z == keys.z {
                        removed.push(placement.image);
                        false
                    } else {
                        true
                    }
                });
                for id in removed {
                    self.release(id, free);
                }
                Area::Z(keys.z)
            }
            _ => {
                return Outcome {
                    reply: reply(keys, Err("ENOTSUPPORTED:delete mode")),
                    place: None,
                    delete: None,
                };
            }
        };
        Outcome {
            reply: None,
            place: None,
            delete: Some(Delete { area, free }),
        }
    }

    /// Frees an image once nothing shows it, which is what the capital delete modes do.
    fn release(&mut self, id: u32, free: bool) {
        if free
            && !self
                .placements
                .values()
                .any(|placement| placement.image == id)
            && !self
                .virtuals
                .values()
                .any(|placement| placement.image == id)
        {
            self.forget(id);
        }
    }

    /// Drops the placements at these anchors, and their image data when `free` is set.
    pub fn drop(&mut self, anchors: &[u32], free: bool) {
        let mut shown = Vec::new();
        for anchor in anchors {
            if let Some(placement) = self.placements.remove(anchor) {
                shown.push(placement.image);
            }
        }
        for id in shown {
            self.release(id, free);
        }
    }

    /// One line in the geometry log, so a capture shows the rect instead of a screenshot.
    pub fn note_placement(&self, request: &Request, column: usize, line: i32) {
        note(&format!(
            "placed id={} at col={column} row={line} cells={}x{} px={}x{}",
            request.image,
            request.columns,
            request.rows,
            request.columns * self.cell.0,
            request.rows * self.cell.1
        ));
    }
}

/// `KORAL_GRAPHICS_LOG=<path>` appends every command as the client sent it, keys and payload
/// length, which is the only way to see what a program actually asks this terminal for.
///
/// `KORAL_FRAME_LOG=<path>` appends one line per frame slower than 8 ms: grid, lock wait, text
/// shaping, glyphon prepare, pictures, acquire, present, line and glyph counts, and the slowest
/// shaped line with its codepoints and face; plus one `warm ...` line when the startup font warm-up
/// runs. Unset, it costs nothing. A diagnostic aid for reporting stutter, not a user setting.
fn trace(control: &[u8], length: usize) {
    note(&format!(
        "{} payload={length}",
        String::from_utf8_lossy(control)
    ));
}

/// One line in the same log, for the geometry the protocol commands work out to.
fn note(line: &str) {
    static LOG: OnceLock<Option<std::fs::File>> = OnceLock::new();
    let Some(file) = LOG.get_or_init(|| {
        let path = std::env::var_os("KORAL_GRAPHICS_LOG")?;
        std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
            .ok()
    }) else {
        return;
    };
    let mut out = file;
    let _ = writeln!(out, "{line}");
}

fn parse(body: &[u8]) -> Option<(Keys, &[u8])> {
    let body = body.strip_prefix(b"G")?;
    let split = body
        .iter()
        .position(|byte| *byte == b';')
        .unwrap_or(body.len());
    let (control, payload) = body.split_at(split);

    let mut keys = Keys::default();
    for pair in std::str::from_utf8(control).ok()?.split(',') {
        let (key, value) = pair.split_once('=')?;
        let number = value.parse().unwrap_or(0);
        let signed = value.parse::<i32>().unwrap_or(0);
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
            "x" => keys.crop[0] = number,
            "y" => keys.crop[1] = number,
            "w" => keys.crop[2] = number,
            "h" => keys.crop[3] = number,
            "d" => keys.delete = letter,
            "m" => keys.more = number == 1,
            "z" => keys.z = signed,
            "U" => keys.virtual_placement = number == 1,
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
    Some(format!(
        "\x1b_Gi={};{}\x1b\\",
        keys.image,
        result.err().unwrap_or("OK")
    ))
}

fn decode(transfer: &Transfer) -> Result<Decoded, &'static str> {
    let keys = transfer.keys;
    if transfer.too_big {
        return Err("EFBIG:image too large");
    }
    let raw = STANDARD_NO_PAD
        .decode(&transfer.data)
        .map_err(|_| "EINVAL:bad base64")?;
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
        raw.as_chunks::<3>()
            .0
            .iter()
            .flat_map(|[r, g, b]| [*r, *g, *b, 255])
            .collect()
    };
    Ok(Decoded {
        width,
        height,
        rgba,
    })
}

fn decode_png(raw: &[u8]) -> Result<Decoded, &'static str> {
    let limits = png::Limits { bytes: MAX_IMAGE };
    let mut decoder = png::Decoder::new_with_limits(Cursor::new(raw), limits);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|_| "EINVAL:bad png")?;
    // The header declares this size: `Limits.bytes` only reserves one row, so refuse before allocating.
    let (width, height) = reader.info().size();
    let size = reader.output_buffer_size().ok_or("EFBIG:image too large")?;
    if u64::from(width) * u64::from(height) * 4 > MAX_IMAGE as u64 || size > MAX_IMAGE {
        return Err("EFBIG:image too large");
    }
    let mut buffer = vec![0; size];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|_| "EINVAL:bad png")?;
    let (width, height) = (info.width, info.height);
    if u64::from(width) * u64::from(height) * 4 > MAX_IMAGE as u64 {
        return Err("EFBIG:image too large");
    }

    let data = &buffer[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|[r, g, b]| [*r, *g, *b, 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => data
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|[v, a]| [*v, *v, *v, *a])
            .collect(),
        png::ColorType::Grayscale => data
            .iter()
            .flat_map(|gray| [*gray, *gray, *gray, 255])
            .collect(),
        png::ColorType::Indexed => return Err("EINVAL:bad png"),
    };
    Ok(Decoded {
        width,
        height,
        rgba,
    })
}

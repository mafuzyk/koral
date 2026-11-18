use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Instant;

use alacritty_terminal::vte::ansi::Rgb;
use glyphon::{
    Attrs, Buffer, Cache, Color, ColorMode, Family, FontSystem, Metrics, Resolution, Shaping, Style,
    SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport, Weight,
};
use winit::window::Window;

use crate::fonts;
use crate::graphics::Image;
use crate::lag;

const LINE_HEIGHT: f32 = 1.2;
const SMALL: f32 = 0.82;
pub const MAX_CACHED_LINES: usize = 4_000;
/// Evicting down to here leaves about twenty screens of headroom, so the next screens fill the cache
/// again instead of evicting on every line.
const EVICT_TO: usize = MAX_CACHED_LINES * 3 / 4;

/// The shaped lines, by a hash of what they say. Bounded by eviction, never by emptying: a screen
/// of unseen lines must not cost the lines of the screen before it, and the frame being drawn keeps
/// its own.
pub struct LineCache<V> {
    entries: HashMap<u64, V>,
    /// Insertion order, the oldest first: what to evict when the cap is reached.
    order: VecDeque<u64>,
    /// Every key shaped in this run, while the frame log is on: it separates new content from
    /// content shaped again after an eviction.
    // shortcut: it grows with the whole run and exists for the log only, so it is off unless asked.
    seen: Option<HashSet<u64>>,
    fresh: usize,
    again: usize,
    evicted: usize,
}

impl<V> Default for LineCache<V> {
    fn default() -> Self {
        Self { entries: HashMap::new(), order: VecDeque::new(), seen: None, fresh: 0, again: 0, evicted: 0 }
    }
}

impl<V> LineCache<V> {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn get(&self, key: &u64) -> Option<&V> {
        self.entries.get(key)
    }

    /// Of this frame's shaping, the lines no earlier frame had shaped.
    pub fn fresh(&self) -> usize {
        self.fresh
    }

    /// Of this frame's shaping, the lines shaped again after an eviction.
    pub fn again(&self) -> usize {
        self.again
    }

    /// Lines this frame's eviction dropped.
    pub fn evicted(&self) -> usize {
        self.evicted
    }

    /// The entry for every key, shaping the ones it lacks with `shape`. Returns how many it shaped:
    /// a key is shaped once however often it repeats in the frame.
    pub fn prepare(&mut self, keys: &[u64], mut shape: impl FnMut(&u64, usize) -> V) -> usize {
        debug_assert_eq!(self.order.len(), self.entries.len(), "the order holds every key once");
        (self.fresh, self.again, self.evicted) = (0, 0, 0);
        let logged = lag::enabled();
        let mut shaped = 0;
        for (index, key) in keys.iter().enumerate() {
            if self.entries.contains_key(key) {
                continue;
            }
            let entry = shape(key, index);
            self.entries.insert(*key, entry);
            self.order.push_back(*key);
            shaped += 1;
            if logged {
                if self.seen.get_or_insert_with(HashSet::new).insert(*key) {
                    self.fresh += 1;
                } else {
                    self.again += 1;
                }
            }
        }
        self.evict(keys);
        shaped
    }

    /// Drops the oldest lines this frame is not showing, down to `EVICT_TO`. A frame larger than the
    /// cap keeps itself whole: those lines would have to be shaped again on the next frame, which is
    /// the cost eviction exists to avoid.
    fn evict(&mut self, frame: &[u64]) {
        if self.entries.len() <= MAX_CACHED_LINES {
            return;
        }
        let shown: HashSet<u64> = frame.iter().copied().collect();
        // One pass at most: a cache of nothing but this frame's lines has nothing to evict.
        for _ in 0..self.order.len() {
            if self.entries.len() <= EVICT_TO {
                break;
            }
            let Some(key) = self.order.pop_front() else { break };
            if shown.contains(&key) {
                self.order.push_back(key);
                continue;
            }
            self.entries.remove(&key);
            self.evicted += 1;
        }
    }
}

const SHADER: &str = r#"
struct Globals { size: vec2<f32>, unused: vec2<f32> };
@group(0) @binding(0) var<uniform> globals: Globals;

struct Instance {
    @builtin(vertex_index) index: u32,
    @location(0) rect: vec4<f32>,
    @location(1) color: vec4<f32>,
    @location(2) shape: vec4<f32>,
};

struct Varying {
    @builtin(position) position: vec4<f32>,
    @location(0) pixel: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) shape: vec4<f32>,
    @location(3) rect: vec4<f32>,
};

@vertex
fn vs_main(in: Instance) -> Varying {
    let corner = vec2<f32>(f32(in.index & 1u), f32((in.index >> 1u) & 1u));
    let pixel = in.rect.xy + corner * in.rect.zw;
    var out: Varying;
    out.position = vec4<f32>(pixel.x / globals.size.x * 2.0 - 1.0, 1.0 - pixel.y / globals.size.y * 2.0, 0.0, 1.0);
    out.pixel = pixel;
    out.color = in.color;
    out.shape = in.shape;
    out.rect = in.rect;
    return out;
}

@fragment
fn fs_main(in: Varying) -> @location(0) vec4<f32> {
    let mode = in.shape.w;
    let radius = in.shape.z;
    var coverage = 1.0;
    if mode > 4.5 {
        // The box ends half a stroke past the radius, which is how the stroke width gets here.
        let half_stroke = in.rect.z - radius;
        let distance = abs(length(in.pixel - in.shape.xy) - radius) - half_stroke;
        coverage = clamp(0.5 - distance, 0.0, 1.0);
    } else if mode > 3.5 {
        let point = in.pixel - in.rect.xy - in.rect.zw * 0.5;
        let distance_from_centre = length(point);
        let sector = 6.2831853 / 8.0;
        let from_tooth = abs(fract(atan2(point.y, point.x) / sector + 0.5) - 0.5) * sector;
        let tooth = max(distance_from_centre * sin(from_tooth) - radius * 0.2, distance_from_centre - radius);
        let body = min(tooth, distance_from_centre - radius * 0.74);
        let distance = max(body, radius * 0.34 - distance_from_centre);
        coverage = clamp(0.5 - distance, 0.0, 1.0);
    } else if mode > 2.5 {
        let half_size = in.rect.zw * 0.5;
        let offset = abs(in.pixel - in.rect.xy - half_size) - half_size + vec2<f32>(radius);
        let distance = length(max(offset, vec2<f32>(0.0))) + min(max(offset.x, offset.y), 0.0) - radius;
        coverage = clamp(0.5 - distance, 0.0, 1.0);
    } else if mode > 0.5 {
        let distance = length(in.pixel - in.shape.xy) - radius;
        coverage = clamp(0.5 - distance, 0.0, 1.0);
        if mode > 1.5 {
            coverage = 1.0 - coverage;
        }
    }
    let alpha = in.color.a * coverage;
    return vec4<f32>(in.color.rgb * alpha, alpha);
}

@group(1) @binding(0) var picture: texture_2d<f32>;
@group(1) @binding(1) var picture_sampler: sampler;

@fragment
fn fs_picture(in: Varying) -> @location(0) vec4<f32> {
    let at = (in.pixel - in.rect.xy) / in.rect.zw;
    let color = textureSample(picture, picture_sampler, in.color.xy + at * in.color.zw);
    return vec4<f32>(color.rgb * color.a, color.a);
}
"#;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    Plain,
    InsideCircle { x: f32, y: f32, radius: f32 },
    OutsideCircle { x: f32, y: f32, radius: f32 },
    Rounded { radius: f32 },
    Gear { radius: f32 },
    Arc { x: f32, y: f32, radius: f32 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quad {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub color: [f32; 4],
    pub shape: Shape,
}

impl Quad {
    pub fn new(x: f32, y: f32, width: f32, height: f32, color: [f32; 4]) -> Self {
        Self { x, y, width, height, color, shape: Shape::Plain }
    }

    fn floats(&self) -> [f32; 12] {
        let [r, g, b, a] = self.color;
        let (cx, cy, radius, mode) = match self.shape {
            Shape::Plain => (0.0, 0.0, 0.0, 0.0),
            Shape::InsideCircle { x, y, radius } => (x, y, radius, 1.0),
            Shape::OutsideCircle { x, y, radius } => (x, y, radius, 2.0),
            Shape::Rounded { radius } => (0.0, 0.0, radius, 3.0),
            Shape::Gear { radius } => (0.0, 0.0, radius, 4.0),
            Shape::Arc { x, y, radius } => (x, y, radius, 5.0),
        };
        [self.x, self.y, self.width, self.height, r, g, b, a, cx, cy, radius, mode]
    }
}

pub fn surface_format(formats: &[wgpu::TextureFormat]) -> wgpu::TextureFormat {
    // Theme colours are sRGB values: a float surface would read them as linear and wash them out.
    let plain = [wgpu::TextureFormat::Bgra8Unorm, wgpu::TextureFormat::Rgba8Unorm];
    plain.into_iter().find(|format| formats.contains(format)).unwrap_or(formats[0])
}

pub fn rgba(color: Rgb, alpha: f32) -> [f32; 4] {
    [f32::from(color.r) / 255.0, f32::from(color.g) / 255.0, f32::from(color.b) / 255.0, alpha]
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Span {
    pub text: String,
    pub color: (u8, u8, u8),
    pub bold: bool,
    pub italic: bool,
}

impl Span {
    pub fn plain(text: impl Into<String>, color: Rgb) -> Self {
        Self { text: text.into(), color: (color.r, color.g, color.b), bold: false, italic: false }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Face {
    pub family: String,
    pub weight: u16,
    pub bold: u16,
    pub bold_italic: u16,
    pub oblique: bool,
}

#[derive(Debug, Clone)]
pub struct TextLine {
    pub x: f32,
    pub y: f32,
    pub small: bool,
    pub spans: Vec<Span>,
}

/// Terminal text under an open panel: the first `count` lines stop at `x` inside the band.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Clip {
    pub count: usize,
    pub x: f32,
    pub top: f32,
    pub bottom: f32,
}

#[derive(Clone)]
pub struct Picture {
    pub image: Arc<Image>,
    /// The image id and placement id, which break ties at the same z-index.
    pub id: u32,
    pub number: u32,
    pub z: i32,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub crop: [f32; 4],
}

/// Below this a picture goes under cells with a background colour, not just under the text.
pub const UNDER_BACKGROUND: i32 = i32::MIN / 2;

/// Pictures in the order they are drawn: under the cell backgrounds first, then under the text,
/// then over everything, each band by z-index, image id and placement id, as kitty orders them.
pub fn layers(list: &[Picture]) -> (Vec<usize>, Vec<usize>, Vec<usize>) {
    let band = |below: bool| -> Vec<usize> {
        let keep = |z: i32| if below { z < UNDER_BACKGROUND } else { z >= UNDER_BACKGROUND && z < 0 };
        let mut found: Vec<usize> = (0..list.len()).filter(|index| keep(list[*index].z)).collect();
        let order = |list: &[Picture], index: &usize| {
            let picture = &list[*index];
            (picture.z, picture.id, picture.number)
        };
        found.sort_by_key(|index| order(list, index));
        found
    };
    let mut above: Vec<usize> = (0..list.len()).filter(|index| list[*index].z >= 0).collect();
    let order = |list: &[Picture], index: &usize| {
        let picture = &list[*index];
        (picture.z, picture.id, picture.number)
    };
    above.sort_by_key(|index| order(list, index));
    (band(true), band(false), above)
}

/// Images go over the first `over` quads and under everything else.
#[derive(Clone, Default)]
pub struct Pictures {
    pub list: Vec<Picture>,
    pub over: usize,
    pub area: [f32; 4],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cell {
    pub width: f32,
    pub height: f32,
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    picture_pipeline: wgpu::RenderPipeline,
    picture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    textures: HashMap<u64, wgpu::BindGroup>,
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    quads: wgpu::Buffer,
    font_system: FontSystem,
    cache: Cache,
    swash: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    text: TextRenderer,
    lines: LineCache<Buffer>,
    wanted: Face,
    face: Face,
    points: f32,
    scale: f32,
    cell: Cell,
    window: Arc<Window>,
}

impl Renderer {
    pub async fn new(window: Arc<Window>, instance: wgpu::Instance, face: &Face, points: f32, mut font_system: FontSystem) -> Self {
        let size = window.inner_size();
        let scale = window.scale_factor() as f32;
        let surface = instance.create_surface(window.clone()).expect("create surface");
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .expect("no graphics adapter");
        let (device, queue) =
            adapter.request_device(&wgpu::DeviceDescriptor::default()).await.expect("request device");

        let capabilities = surface.get_capabilities(&adapter);
        let format = surface_format(&capabilities.formats);
        let alpha_mode = [wgpu::CompositeAlphaMode::PreMultiplied, wgpu::CompositeAlphaMode::PostMultiplied]
            .into_iter()
            .find(|mode| capabilities.alpha_modes.contains(mode))
            .unwrap_or(capabilities.alpha_modes[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("quads"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() }],
        });
        let picture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline = pipeline(&device, &shader, format, &[Some(&layout)], "fs_main");
        let picture_pipeline =
            self::pipeline(&device, &shader, format, &[Some(&layout), Some(&picture_layout)], "fs_picture");
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let quads = quad_buffer(&device, 1024);

        let cache = Cache::new(&device);
        let viewport = Viewport::new(&device, &cache);
        let mut atlas = TextAtlas::with_color_mode(&device, &queue, &cache, format, ColorMode::Web);
        let text = TextRenderer::new(&mut atlas, &device, wgpu::MultisampleState::default(), None);
        let wanted = face.clone();
        let face = fonts::usable(face, font_system.db());
        let cell = measure(&mut font_system, &face, points, scale);

        Self {
            device,
            queue,
            instance,
            surface,
            config,
            pipeline,
            picture_pipeline,
            picture_layout,
            sampler,
            textures: HashMap::new(),
            globals,
            bind_group,
            quads,
            font_system,
            cache,
            swash: SwashCache::new(),
            viewport,
            atlas,
            text,
            lines: LineCache::default(),
            wanted,
            face,
            points,
            scale,
            cell,
            window,
        }
    }

    pub fn cell(&self) -> Cell {
        self.cell
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(&self.device, &self.config);
    }

    pub fn set_scale(&mut self, scale: f32) {
        self.scale = scale;
        self.remeasure();
    }

    /// The full font list. Font ids differ between systems, so nothing shaped or cached carries over.
    pub fn set_fonts(&mut self, fonts: FontSystem) {
        self.font_system = fonts;
        self.swash = SwashCache::new();
        self.atlas = TextAtlas::with_color_mode(&self.device, &self.queue, &self.cache, self.config.format, ColorMode::Web);
        self.text = TextRenderer::new(&mut self.atlas, &self.device, wgpu::MultisampleState::default(), None);
        self.face = fonts::usable(&self.wanted, self.font_system.db());
        self.remeasure();
    }

    pub fn set_font(&mut self, face: &Face, points: f32) {
        if *face != self.wanted {
            self.wanted = face.clone();
            self.face = fonts::usable(face, self.font_system.db());
        }
        self.points = points;
        self.remeasure();
    }

    /// The family drawn in place of the one in the config.
    pub fn substitute(&self) -> Option<&str> {
        (self.face.family != self.wanted.family).then_some(self.face.family.as_str())
    }

    fn remeasure(&mut self) {
        self.cell = measure(&mut self.font_system, &self.face, self.points, self.scale);
        self.lines = LineCache::default();
    }

    fn upload(&mut self, image: &Image) {
        let size = wgpu::Extent3d { width: image.width, height: image.height, depth_or_array_layers: 1 };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("picture"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // Not an sRGB format: the surface takes plain values, see `surface_format`.
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            texture.as_image_copy(),
            &image.rgba,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(image.width * 4), rows_per_image: None },
            size,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.picture_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        self.textures.insert(image.generation, bind_group);
    }

    /// Fills `cost` with the phases of the frame, for `lag::note`.
    pub fn draw(&mut self, quads: &[Quad], pictures: &Pictures, lines: &[TextLine], clip: Clip, clear: [f32; 4], cost: &mut lag::Cost) {
        let text_from = Instant::now();
        cost.lines = lines.len();
        let keys: Vec<u64> = lines.iter().map(line_key).collect();
        let font_system = &mut self.font_system;
        let (face, points, scale) = (&self.face, self.points, self.scale);
        let timed = lag::enabled();
        let mut worst = lag::LineCost::default();
        let shape_from = Instant::now();
        cost.shaped += self.lines.prepare(&keys, |_, index| {
            let line = &lines[index];
            let start = timed.then(Instant::now);
            let buffer = shape(font_system, line, face, points, scale);
            if let Some(start) = start {
                let micros = start.elapsed().as_micros() as u64;
                if micros > worst.micros {
                    worst = describe(line, micros);
                }
            }
            buffer
        });
        if timed {
            cost.shape = lag::millis(shape_from);
            cost.worst = worst;
            (cost.fresh, cost.again, cost.evicted) = (self.lines.fresh(), self.lines.again(), self.lines.evicted());
        }

        let (width, height) = (self.config.width, self.config.height);
        self.viewport.update(&self.queue, Resolution { width, height });
        let bounds = TextBounds { left: 0, top: 0, right: width as i32, bottom: height as i32 };
        let cached = &self.lines;
        let areas = keys.iter().zip(lines).enumerate().map(|(index, (key, line))| {
            let clipped = index < clip.count && (clip.top..clip.bottom).contains(&line.y);
            TextArea {
                buffer: cached.get(key).expect("prepare holds every line of the frame"),
                left: line.x,
                top: line.y,
                scale: 1.0,
                bounds: if clipped { TextBounds { right: clip.x as i32, ..bounds } } else { bounds },
                default_color: Color::rgb(255, 255, 255),
                custom_glyphs: &[],
            }
        });
        let prepare_from = Instant::now();
        let prepared = self.text.prepare(
            &self.device,
            &self.queue,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            areas,
            &mut self.swash,
        );
        cost.prepare = lag::millis(prepare_from);
        cost.text = lag::millis(text_from);
        if prepared.is_err() {
            self.atlas.trim();
            self.window.request_redraw();
            return;
        }

        let acquire_from = Instant::now();
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                self.window.request_redraw();
                return;
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Suboptimal(_) => {
                self.surface.configure(&self.device, &self.config);
                self.window.request_redraw();
                return;
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.surface = self.instance.create_surface(self.window.clone()).expect("recreate surface");
                self.surface.configure(&self.device, &self.config);
                self.window.request_redraw();
                return;
            }
            wgpu::CurrentSurfaceTexture::Validation => panic!("surface validation error"),
        };
        cost.acquire = lag::millis(acquire_from);

        let globals: Vec<u8> = [width as f32, height as f32, 0.0, 0.0]
            .iter()
            .flat_map(|value| value.to_ne_bytes())
            .collect();
        self.queue.write_buffer(&self.globals, 0, &globals);

        let pictures_from = Instant::now();
        // shortcut: an image over the texture limit is not drawn, scale it down if one shows up.
        let limit = self.device.limits().max_texture_dimension_2d;
        let drawn: Vec<Picture> = pictures
            .list
            .iter()
            .filter(|picture| picture.image.width.max(picture.image.height) <= limit)
            .cloned()
            .collect();
        let used: HashSet<u64> = drawn.iter().map(|picture| picture.image.generation).collect();
        self.textures.retain(|generation, _| used.contains(generation));
        for picture in &drawn {
            if !self.textures.contains_key(&picture.image.generation) {
                self.upload(&picture.image);
                cost.uploads += 1;
            }
        }

        let (below, under, above) = layers(&drawn);
        cost.pictures = lag::millis(pictures_from);

        let present_from = Instant::now();
        let boxes = drawn.iter().map(|p| Quad::new(p.x, p.y, p.width, p.height, p.crop));
        let instances: Vec<u8> = quads
            .iter()
            .copied()
            .chain(boxes)
            .flat_map(|quad| quad.floats())
            .flat_map(|value| value.to_ne_bytes())
            .collect();
        if instances.len() as u64 > self.quads.size() {
            self.quads = quad_buffer(&self.device, (quads.len() + drawn.len()) * 2);
        }
        self.queue.write_buffer(&self.quads, 0, &instances);

        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let [r, g, b, a] = clear;
            let (r, g, b, a) = (f64::from(r), f64::from(g), f64::from(b), f64::from(a));
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: r * a, g: g * a, b: b * a, a }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.quads.slice(..));
            let (count, over) = (quads.len() as u32, pictures.over as u32);
            // One z layer of pictures, clipped to the grid so a picture wider than the view is cut
            // by the view and not sized by it.
            let layer = |target: &mut wgpu::RenderPass<'_>, order: &[usize]| {
                if order.is_empty() {
                    return;
                }
                let [x, y, area_width, area_height] = pictures.area;
                let (left, top) = ((x as u32).min(width), (y as u32).min(height));
                let right = ((x + area_width) as u32).min(width);
                let bottom = ((y + area_height) as u32).min(height);
                target.set_scissor_rect(left, top, right.saturating_sub(left), bottom.saturating_sub(top));
                target.set_pipeline(&self.picture_pipeline);
                // The band after the text inherits glyphon's bind group 0 and vertex buffer unless
                // each band binds its own: the atlas layout and stride are not ours.
                target.set_bind_group(0, &self.bind_group, &[]);
                target.set_vertex_buffer(0, self.quads.slice(..));
                for index in order {
                    let picture = &drawn[*index];
                    target.set_bind_group(1, &self.textures[&picture.image.generation], &[]);
                    let at = count + *index as u32;
                    target.draw(0..4, at..at + 1);
                }
                target.set_scissor_rect(0, 0, width, height);
                target.set_pipeline(&self.pipeline);
            };
            layer(&mut pass, &below);
            pass.draw(0..4, 0..over);
            layer(&mut pass, &under);
            pass.draw(0..4, over..count);
            self.text.render(&self.atlas, &self.viewport, &mut pass).expect("render text");
            layer(&mut pass, &above);
        }
        self.queue.submit(Some(encoder.finish()));
        self.window.pre_present_notify();
        self.queue.present(frame);
        cost.present = lag::millis(present_from);
        self.atlas.trim();
    }
}

fn pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
    groups: &[Option<&wgpu::BindGroupLayout>],
    fragment: &str,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: groups,
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(fragment),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: 48,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &wgpu::vertex_attr_array![
                    0 => Float32x4,
                    1 => Float32x4,
                    2 => Float32x4,
                ],
            })],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn quad_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("quads"),
        size: capacity as u64 * 48,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn metrics(points: f32, scale: f32, small: bool) -> Metrics {
    let pixels = points * 96.0 / 72.0 * scale;
    Metrics::new(pixels * if small { SMALL } else { 1.0 }, (pixels * LINE_HEIGHT).round())
}

pub fn attrs(face: &Face, bold: bool, italic: bool) -> Attrs<'_> {
    let weight = match (bold, italic) {
        (true, true) => face.bold_italic,
        (true, false) => face.bold,
        (false, _) => face.weight,
    };
    let attrs = Attrs::new().family(Family::Name(&face.family)).weight(Weight(weight));
    match (italic, face.oblique) {
        (false, _) => attrs,
        (true, true) => attrs.style(Style::Oblique),
        (true, false) => attrs.style(Style::Italic),
    }
}

fn measure(font_system: &mut FontSystem, face: &Face, points: f32, scale: f32) -> Cell {
    let metrics = metrics(points, scale, false);
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_text("M", &attrs(face, false, false), Shaping::Advanced, None);
    buffer.shape_until_scroll(font_system, false);
    let width = buffer.layout_runs().next().map_or(metrics.font_size * 0.5, |run| run.line_w);
    Cell { width, height: metrics.line_height }
}

fn shape(font_system: &mut FontSystem, line: &TextLine, face: &Face, points: f32, scale: f32) -> Buffer {
    let mut buffer = Buffer::new(font_system, metrics(points, scale, line.small));
    let spans = line.spans.iter().map(|span| {
        let (r, g, b) = span.color;
        (span.text.as_str(), attrs(face, span.bold, span.italic).color(Color::rgb(r, g, b)))
    });
    buffer.set_rich_text(spans, &attrs(face, false, false), Shaping::Advanced, None);
    buffer.shape_until_scroll(font_system, false);
    buffer
}

fn line_key(line: &TextLine) -> u64 {
    let mut hasher = DefaultHasher::new();
    line.small.hash(&mut hasher);
    line.spans.hash(&mut hasher);
    hasher.finish()
}

/// What the frame log needs of the slowest line: its cost and what it is made of.
fn describe(line: &TextLine, micros: u64) -> lag::LineCost {
    let mut colors: Vec<(u8, u8, u8)> = line.spans.iter().map(|span| span.color).collect();
    colors.sort_unstable();
    colors.dedup();
    lag::LineCost {
        micros,
        chars: line.spans.iter().map(|span| span.text.chars().count()).sum(),
        spans: line.spans.len(),
        colors: colors.len(),
        non_ascii: line.spans.iter().any(|span| !span.text.is_ascii()),
    }
}

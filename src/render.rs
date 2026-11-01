use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use alacritty_terminal::vte::ansi::Rgb;
use glyphon::{
    Attrs, Buffer, Cache, Color, ColorMode, Family, FontSystem, Metrics, Resolution, Shaping, Style,
    SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport, Weight,
};
use winit::window::Window;

const LINE_HEIGHT: f32 = 1.2;
const SMALL: f32 = 0.82;
const MAX_CACHED_LINES: usize = 4_000;

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
    if mode > 3.5 {
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
"#;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    Plain,
    InsideCircle { x: f32, y: f32, radius: f32 },
    OutsideCircle { x: f32, y: f32, radius: f32 },
    Rounded { radius: f32 },
    Gear { radius: f32 },
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
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    quads: wgpu::Buffer,
    font_system: FontSystem,
    swash: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    text: TextRenderer,
    lines: HashMap<u64, Buffer>,
    font: String,
    points: f32,
    scale: f32,
    cell: Cell,
    window: Arc<Window>,
}

impl Renderer {
    pub async fn new(window: Arc<Window>, instance: wgpu::Instance, font: String, points: f32) -> Self {
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
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("quads"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
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
                module: &shader,
                entry_point: Some("fs_main"),
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
        });
        let quads = quad_buffer(&device, 1024);

        let mut font_system = FontSystem::new();
        let cache = Cache::new(&device);
        let viewport = Viewport::new(&device, &cache);
        let mut atlas = TextAtlas::with_color_mode(&device, &queue, &cache, format, ColorMode::Web);
        let text = TextRenderer::new(&mut atlas, &device, wgpu::MultisampleState::default(), None);
        let cell = measure(&mut font_system, &font, points, scale);

        Self {
            device,
            queue,
            instance,
            surface,
            config,
            pipeline,
            globals,
            bind_group,
            quads,
            font_system,
            swash: SwashCache::new(),
            viewport,
            atlas,
            text,
            lines: HashMap::new(),
            font,
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

    pub fn set_font(&mut self, font: &str, points: f32) {
        self.font = font.to_string();
        self.points = points;
        self.remeasure();
    }

    fn remeasure(&mut self) {
        self.cell = measure(&mut self.font_system, &self.font, self.points, self.scale);
        self.lines.clear();
    }

    pub fn draw(&mut self, quads: &[Quad], lines: &[TextLine], clip: Clip, clear: [f32; 4]) {
        let keys: Vec<u64> = lines.iter().map(line_key).collect();
        if self.lines.len() + keys.len() > MAX_CACHED_LINES {
            self.lines.clear();
        }
        for (key, line) in keys.iter().zip(lines) {
            if !self.lines.contains_key(key) {
                let buffer = shape(&mut self.font_system, line, &self.font, self.points, self.scale);
                self.lines.insert(*key, buffer);
            }
        }

        let (width, height) = (self.config.width, self.config.height);
        self.viewport.update(&self.queue, Resolution { width, height });
        let bounds = TextBounds { left: 0, top: 0, right: width as i32, bottom: height as i32 };
        let cached = &self.lines;
        let areas = keys.iter().zip(lines).enumerate().map(|(index, (key, line))| {
            let clipped = index < clip.count && (clip.top..clip.bottom).contains(&line.y);
            TextArea {
                buffer: &cached[key],
                left: line.x,
                top: line.y,
                scale: 1.0,
                bounds: if clipped { TextBounds { right: clip.x as i32, ..bounds } } else { bounds },
                default_color: Color::rgb(255, 255, 255),
                custom_glyphs: &[],
            }
        });
        if self
            .text
            .prepare(
                &self.device,
                &self.queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                areas,
                &mut self.swash,
            )
            .is_err()
        {
            self.atlas.trim();
            self.window.request_redraw();
            return;
        }

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

        let globals: Vec<u8> = [width as f32, height as f32, 0.0, 0.0]
            .iter()
            .flat_map(|value| value.to_ne_bytes())
            .collect();
        self.queue.write_buffer(&self.globals, 0, &globals);

        let instances: Vec<u8> =
            quads.iter().flat_map(Quad::floats).flat_map(|value| value.to_ne_bytes()).collect();
        if instances.len() as u64 > self.quads.size() {
            self.quads = quad_buffer(&self.device, quads.len() * 2);
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
            pass.draw(0..4, 0..quads.len() as u32);
            self.text.render(&self.atlas, &self.viewport, &mut pass).expect("render text");
        }
        self.queue.submit(Some(encoder.finish()));
        self.window.pre_present_notify();
        self.queue.present(frame);
        self.atlas.trim();
    }
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

fn base_attrs(font: &str) -> Attrs<'_> {
    Attrs::new().family(Family::Name(font)).weight(Weight::MEDIUM)
}

fn measure(font_system: &mut FontSystem, font: &str, points: f32, scale: f32) -> Cell {
    let metrics = metrics(points, scale, false);
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_text("M", &base_attrs(font), Shaping::Advanced, None);
    buffer.shape_until_scroll(font_system, false);
    let width = buffer.layout_runs().next().map_or(metrics.font_size * 0.5, |run| run.line_w);
    Cell { width, height: metrics.line_height }
}

fn shape(font_system: &mut FontSystem, line: &TextLine, font: &str, points: f32, scale: f32) -> Buffer {
    let mut buffer = Buffer::new(font_system, metrics(points, scale, line.small));
    let spans = line.spans.iter().map(|span| {
        let (r, g, b) = span.color;
        let mut attrs = base_attrs(font).color(Color::rgb(r, g, b));
        if span.bold {
            attrs = attrs.weight(Weight::BOLD);
        }
        if span.italic {
            attrs = attrs.style(Style::Italic);
        }
        (span.text.as_str(), attrs)
    });
    buffer.set_rich_text(spans, &base_attrs(font), Shaping::Advanced, None);
    buffer.shape_until_scroll(font_system, false);
    buffer
}

fn line_key(line: &TextLine) -> u64 {
    let mut hasher = DefaultHasher::new();
    line.small.hash(&mut hasher);
    line.spans.hash(&mut hasher);
    hasher.finish()
}

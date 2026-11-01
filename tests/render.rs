use std::time::Instant;

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::Rgb;
use koral::frame::{CursorView, Frame, Underline, draw_terminal, underline};
use glyphon::{Style, Weight};
use koral::render::{Face, Quad, attrs, rgba, surface_format};
use koral::session::Session;
use koral::theme::Theme;
use koral::view::rows;
use wgpu::TextureFormat::{Bgra8Unorm, Bgra8UnormSrgb, Rgba8Unorm, Rgba16Float};

#[test]
fn a_float_surface_is_not_chosen_because_its_colours_are_linear() {
    assert_eq!(surface_format(&[Rgba16Float, Bgra8UnormSrgb, Bgra8Unorm]), Bgra8Unorm);
}

#[test]
fn a_surface_without_bgra_uses_rgba() {
    assert_eq!(surface_format(&[Rgba16Float, Rgba8Unorm]), Rgba8Unorm);
}

#[test]
fn a_picture_keeps_its_shape_inside_its_cells() {
    assert_eq!(koral::frame::fit_picture((100, 50), (100.0, 60.0)), (100.0, 50.0));
    assert_eq!(koral::frame::fit_picture((16, 16), (8.0, 24.0)), (8.0, 8.0));
}

fn layout() -> koral::frame::Layout {
    let cell = koral::render::Cell { width: 10.0, height: 20.0 };
    koral::frame::Layout { width: 800.0, height: 600.0, scale: 1.0, padding: 0.0, cell }
}

#[test]
fn preedit_text_is_underlined_at_the_cursor() {
    let mut frame = koral::frame::Frame::default();
    let theme = koral::theme::Theme::default();
    koral::frame::draw_preedit(&mut frame, "ãé", 30.0, 40.0, layout(), &theme);
    assert_eq!(frame.lines.len(), 1);
    assert_eq!((frame.lines[0].x, frame.lines[0].y), (30.0, 40.0));
    assert_eq!(frame.lines[0].spans[0].text, "ãé");
    assert_eq!(frame.quads.len(), 1);
}

const INK: [f32; 4] = [1.0; 4];

fn drawn(text: &str) -> Frame {
    let term = Term::new(Config::default(), &TermSize::new(20, 5), VoidListener);
    let mut session = Session::new(term);
    session.feed(text.as_bytes(), Instant::now());
    let rows = rows(&session.term, &session.blocks);
    let mut frame = Frame::default();
    let cursor = CursorView { focused: true, visible: true };
    draw_terminal(&mut frame, &session.term, &session.blocks, &rows, 0, cursor, layout(), &Theme::default());
    frame
}

fn covered(quads: &[Quad]) -> f32 {
    quads.iter().map(|quad| quad.width).sum()
}

#[test]
fn single_is_one_quad() {
    assert_eq!(underline(Underline::Single, 0.0, 10.0, 8.0, 1.0, INK).len(), 1);
}

#[test]
fn double_is_two_quads_apart() {
    let quads = underline(Underline::Double, 0.0, 10.0, 8.0, 1.0, INK);
    assert_eq!(quads.len(), 2);
    assert_eq!((quads[1].y - quads[0].y).abs(), 2.0);
}

#[test]
fn dashed_and_dotted_leave_gaps() {
    for style in [Underline::Dashed, Underline::Dotted] {
        let quads = underline(style, 0.0, 10.0, 40.0, 1.0, INK);
        assert!(quads.len() > 1);
        assert!(covered(&quads) < 40.0);
    }
}

#[test]
fn curl_covers_the_whole_width() {
    let quads = underline(Underline::Curl, 3.0, 10.0, 12.0, 1.0, INK);
    assert_eq!(covered(&quads), 12.0);
    assert!(quads.iter().all(|quad| (quad.y - 10.0).abs() <= 1.0));
    assert!(quads.iter().any(|quad| quad.y != 10.0));
}

#[test]
fn curl_joins_between_cells() {
    let first = underline(Underline::Curl, 0.0, 10.0, 10.0, 1.0, INK);
    let second = underline(Underline::Curl, 10.0, 10.0, 10.0, 1.0, INK);
    assert!((first[first.len() - 1].y - second[0].y).abs() <= 1.0);
}

#[test]
fn an_underline_takes_its_own_colour() {
    let red = rgba(Rgb { r: 255, g: 0, b: 0 }, 1.0);
    assert!(drawn("\x1b[4:3;58:2::255:0:0mx").quads.iter().any(|quad| quad.color == red));
}

#[test]
fn a_drawn_box_leaves_no_glyph() {
    let frame = drawn("a\u{2502}b");
    let text: String = frame.lines.iter().flat_map(|line| &line.spans).map(|span| span.text.as_str()).collect();
    assert_eq!(text, "a b");
    assert!(frame.quads.iter().any(|quad| quad.height == 20.0 && quad.x > 0.0));
}

#[test]
fn bold_italic_takes_the_chosen_weight_and_style() {
    let face = Face { family: "Iosevka".to_string(), weight: 500, bold: 800, bold_italic: 900, oblique: true };
    let attrs = attrs(&face, true, true);
    assert_eq!(attrs.weight, Weight(900));
    assert_eq!(attrs.style, Style::Oblique);
}

#[test]
fn plain_text_takes_the_regular_weight() {
    let face = Face { family: "Iosevka".to_string(), weight: 500, bold: 800, bold_italic: 900, oblique: false };
    assert_eq!(attrs(&face, false, false).weight, Weight(500));
    assert_eq!(attrs(&face, true, false).weight, Weight(800));
    assert_eq!(attrs(&face, false, true).style, Style::Italic);
}

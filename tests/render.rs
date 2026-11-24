use std::time::Instant;

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::index::Column;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::Rgb;
use glyphon::{Style, Weight};
use koral::blocks::Activity;
use koral::fonts::Coverage;
use koral::frame::{
    Chrome, CursorView, Frame, TabView, Underline, draw_chrome, draw_terminal, draw_terminal_in,
    underline,
};
use koral::render::{Face, LineCache, MAX_CACHED_LINES, Quad, attrs, rgba, surface_format};
use koral::session::Session;
use koral::theme::Theme;
use koral::view::rows;
use wgpu::TextureFormat::{Bgra8Unorm, Bgra8UnormSrgb, Rgba8Unorm, Rgba16Float};

#[test]
fn a_float_surface_is_not_chosen_because_its_colours_are_linear() {
    assert_eq!(
        surface_format(&[Rgba16Float, Bgra8UnormSrgb, Bgra8Unorm]),
        Bgra8Unorm
    );
}

#[test]
fn a_surface_without_bgra_uses_rgba() {
    assert_eq!(surface_format(&[Rgba16Float, Rgba8Unorm]), Rgba8Unorm);
}

/// The cells are the destination: kitty maps the whole source crop into them.
#[test]
fn a_picture_fills_the_cells_it_was_given() {
    let (size, crop) = koral::frame::picture_box((100, 50), [0, 0, 100, 50], (100.0, 60.0));
    assert_eq!(
        size,
        (100.0, 60.0),
        "the cells win over the image's own shape"
    );
    assert_eq!(crop, [0.0, 0.0, 1.0, 1.0]);
}

#[test]
fn a_cropped_tile_fills_its_cells_so_no_seam_shows() {
    let (size, crop) = koral::frame::picture_box((100, 50), [25, 10, 50, 20], (9.0, 21.0));
    assert_eq!(size, (9.0, 21.0));
    assert_eq!(crop, [0.25, 0.2, 0.5, 0.4]);
}

fn layout() -> koral::frame::Layout {
    let cell = koral::render::Cell {
        width: 10.0,
        height: 20.0,
    };
    koral::frame::Layout {
        width: 800.0,
        height: 600.0,
        scale: 1.0,
        padding: 0.0,
        cell,
    }
}

/// The cells a screenshot is drawn in, at the size the placement math assumes. The window height
/// matches the forty rows the fixtures use, so the first row is the top of the view.
fn picture_layout() -> koral::frame::Layout {
    let cell = koral::render::Cell {
        width: 9.0,
        height: 22.0,
    };
    koral::frame::Layout {
        width: 1200.0,
        height: 918.0,
        scale: 1.0,
        padding: 0.0,
        cell,
    }
}

fn deflated(width: u32, height: u32) -> String {
    use std::io::Write;

    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder
        .write_all(&vec![180; (width * height * 4) as usize])
        .unwrap();
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(encoder.finish().unwrap())
}

/// Transmits a solid picture and places it, then draws the frame, so the rect can be read back.
fn picture_frame(width: u32, height: u32, place: &str, layout: koral::frame::Layout) -> Frame {
    let term = Term::new(Config::default(), &TermSize::new(140, 40), VoidListener);
    let mut session = Session::new(term);
    session
        .graphics
        .set_cell(layout.cell.width as u32, layout.cell.height as u32);
    let payload = deflated(width, height);
    let transmit = format!("\x1b_Ga=t,f=32,s={width},v={height},o=z,i=1,q=2;{payload}\x1b\\");
    let place = format!("\x1b_Ga=p,i=1,{place}\x1b\\");
    session.feed(transmit.as_bytes(), Instant::now());
    session.feed(place.as_bytes(), Instant::now());
    let rows = rows(&session.term, &session.blocks);
    let mut frame = Frame::default();
    koral::frame::draw_pictures(
        &mut frame,
        &session.term,
        &session.graphics,
        &rows,
        0,
        layout,
    );
    frame
}

#[test]
fn a_large_picture_is_drawn_at_the_cells_it_was_given() {
    let layout = picture_layout();
    let frame = picture_frame(1920, 1080, "c=100,r=40,C=1,q=2", layout);
    let picture = &frame.pictures.list[0];
    assert_eq!((picture.width, picture.height), (900.0, 880.0));
    assert_eq!((picture.x, picture.y), (layout.grid_x(), layout.grid_y()));
}

#[test]
fn a_picture_smaller_than_its_cells_is_scaled_up() {
    let frame = picture_frame(100, 50, "c=100,r=40,C=1,q=2", picture_layout());
    let picture = &frame.pictures.list[0];
    assert_eq!(
        (picture.width, picture.height),
        (900.0, 880.0),
        "a small picture fills its cells"
    );
    assert_eq!(
        (picture.x, picture.y),
        (picture_layout().grid_x(), picture_layout().grid_y())
    );
}

#[test]
fn a_picture_past_the_last_column_is_cut_by_the_view_not_sized_by_it() {
    let layout = picture_layout();
    let frame = picture_frame(1920, 1080, "c=200,r=40,C=1,q=2", layout);
    let picture = &frame.pictures.list[0];
    assert_eq!(
        picture.width, 1800.0,
        "the cells decide, not the picture's own width"
    );
    let [_, _, area_width, _] = frame.pictures.area;
    assert!(
        picture.x + picture.width > layout.grid_x() + area_width,
        "the viewport is the clip"
    );
}

#[test]
fn only_columns_gives_the_rows_the_image_aspect_needs() {
    let layout = picture_layout();
    let frame = picture_frame(1920, 1080, "c=50,C=1,q=2", layout);
    let picture = &frame.pictures.list[0];
    assert_eq!((picture.width, picture.height), (450.0, 264.0));
}

fn picture(id: u32, number: u32, z: i32) -> koral::render::Picture {
    let image = koral::graphics::Image {
        generation: u64::from(id),
        width: 1,
        height: 1,
        rgba: vec![0; 4],
    };
    koral::render::Picture {
        image: std::sync::Arc::new(image),
        id,
        number,
        z,
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
        crop: [0.0, 0.0, 1.0, 1.0],
        clip: None,
    }
}

#[test]
fn a_negative_z_picture_is_under_the_text_and_the_rest_over_it() {
    let list = [picture(4, 0, 0), picture(2, 0, -5), picture(9, 0, 3)];
    let (below, under, above) = koral::render::layers(&list);
    assert!(below.is_empty());
    assert_eq!(under, vec![1], "under the text layer");
    assert_eq!(above, vec![0, 2], "over the text layer");
}

#[test]
fn a_picture_under_the_background_band_goes_below_the_cells() {
    let list = [
        picture(4, 0, -1),
        picture(5, 0, i32::MIN),
        picture(6, 0, koral::render::UNDER_BACKGROUND),
    ];
    let (below, under, above) = koral::render::layers(&list);
    assert_eq!(
        below,
        vec![1],
        "z below INT32_MIN/2 sinks under the cell backgrounds"
    );
    assert_eq!(under, vec![2, 0], "the bound itself stays above the cells");
    assert!(above.is_empty());
}

#[test]
fn pictures_at_one_z_stack_by_image_then_placement_id() {
    let list = [
        picture(7, 2, -5),
        picture(3, 9, -9),
        picture(2, 4, -5),
        picture(2, 3, -5),
    ];
    let (_, under, _) = koral::render::layers(&list);
    assert_eq!(under, vec![1, 3, 2, 0]);
}

#[test]
fn preedit_text_is_underlined_at_the_cursor() {
    let mut frame = koral::frame::Frame::default();
    let theme = koral::theme::Theme::default();
    koral::frame::draw_preedit(&mut frame, "ãé", 30.0, 40.0, layout(), &theme);
    assert_eq!(frame.lines.len(), 1);
    assert_eq!((frame.lines[0].x, frame.lines[0].y), (30.0, 40.0));
    assert_eq!(frame.lines[0].spans[0].text, "ãé");
    assert_eq!(frame.quads.len(), 2);
    assert_eq!(frame.quads[0].color, rgba(theme.background, 1.0));
}

const INK: [f32; 4] = [1.0; 4];

fn drawn(text: &str) -> Frame {
    let term = Term::new(Config::default(), &TermSize::new(20, 5), VoidListener);
    let mut session = Session::new(term);
    session.feed(text.as_bytes(), Instant::now());
    let rows = rows(&session.term, &session.blocks);
    let mut frame = Frame::default();
    let cursor = CursorView {
        focused: true,
        visible: true,
    };
    draw_terminal(
        &mut frame,
        &session.term,
        &session.blocks,
        &rows,
        0,
        cursor,
        layout(),
        &Theme::default(),
    );
    frame
}

fn drawn_with_caret(text: &str, column: usize) -> Frame {
    let term = Term::new(Config::default(), &TermSize::new(20, 5), VoidListener);
    let mut session = Session::new(term);
    session.feed(text.as_bytes(), Instant::now());
    session.term.grid_mut().cursor.point.column = Column(column);
    let rows = rows(&session.term, &session.blocks);
    let mut frame = Frame::default();
    let cursor = CursorView {
        focused: true,
        visible: true,
    };
    draw_terminal(
        &mut frame,
        &session.term,
        &session.blocks,
        &rows,
        0,
        cursor,
        layout(),
        &Theme::default(),
    );
    frame
}

fn covered(quads: &[Quad]) -> f32 {
    quads.iter().map(|quad| quad.width).sum()
}

fn cell(width: f32, height: f32) -> koral::render::Cell {
    koral::render::Cell { width, height }
}

#[test]
fn decorations_grow_with_the_cell() {
    let (thin, offset) = koral::frame::decoration(cell(8.0, 20.0));
    let (thick, _) = koral::frame::decoration(cell(16.0, 40.0));
    assert!(thin >= 1.0);
    assert!(thick > thin);
    assert!(offset >= thin);
    assert!(offset + thin <= 20.0);
}

#[test]
fn a_strike_line_sits_at_the_middle_of_the_cell() {
    assert_eq!(koral::frame::strike(cell(10.0, 20.0)), 10.0);
    assert_eq!(koral::frame::strike(cell(10.0, 34.0)), 17.0);
}

#[test]
fn an_underline_hangs_above_the_cell_bottom() {
    let frame = drawn("\x1b[4mx");
    let (thickness, offset) = koral::frame::decoration(layout().cell);
    let expected = layout().grid_y() + layout().cell.height - offset;
    assert!(
        frame
            .quads
            .iter()
            .any(|quad| quad.y == expected && quad.height == thickness)
    );
}

#[test]
fn a_strikeout_crosses_the_letters() {
    let frame = drawn("\x1b[9mx");
    let (thickness, _) = koral::frame::decoration(layout().cell);
    let expected = layout().grid_y() + koral::frame::strike(layout().cell);
    assert!(
        frame
            .quads
            .iter()
            .any(|quad| quad.y == expected && quad.height == thickness)
    );
}

#[test]
fn every_underline_style_draws_a_different_line() {
    let marks = [
        Underline::Single,
        Underline::Double,
        Underline::Curl,
        Underline::Dotted,
        Underline::Dashed,
    ]
    .map(|style| underline(style, 0.0, 10.0, 40.0, 1.0, INK).len());
    assert_eq!(marks, [1, 2, 40, 20, 7]);
}

#[test]
fn a_hint_keeps_its_point() {
    assert_eq!(
        koral::frame::brief("Can't use Missing. Showing Iosevka."),
        "Can't use Missing."
    );
    assert_eq!(
        koral::frame::brief("Not saved: no config folder"),
        "Not saved: no config folder"
    );
    assert_eq!(
        koral::frame::brief("Not saved: disk full. Retry."),
        "Not saved: disk full. Retry."
    );
}

#[test]
fn single_is_one_quad() {
    assert_eq!(
        underline(Underline::Single, 0.0, 10.0, 8.0, 1.0, INK).len(),
        1
    );
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
    assert!(
        drawn("\x1b[4:3;58:2::255:0:0mx")
            .quads
            .iter()
            .any(|quad| quad.color == red)
    );
}

#[test]
fn a_drawn_box_leaves_no_glyph() {
    let frame = drawn("a\u{2502}b");
    let text: String = frame
        .lines
        .iter()
        .flat_map(|line| &line.spans)
        .map(|span| span.text.as_str())
        .collect();
    assert_eq!(text, "a b");
    assert!(
        frame
            .quads
            .iter()
            .any(|quad| quad.height == 20.0 && quad.x > 0.0)
    );
}

#[test]
fn a_block_cursor_covers_exactly_one_cell() {
    let frame = drawn("x");
    let cursor = layout().grid_x() + layout().cell.width;
    assert!(frame.quads.iter().any(|quad| {
        quad.x == cursor && quad.width == layout().cell.width && quad.height == layout().cell.height
    }));
}

#[test]
fn a_wide_glyph_takes_a_two_cell_cursor() {
    let frame = drawn_with_caret("日", 0);
    assert!(frame.quads.iter().any(|quad| {
        quad.x == layout().grid_x()
            && quad.width == 2.0 * layout().cell.width
            && quad.height == layout().cell.height
    }));
    assert!(
        frame
            .quads
            .iter()
            .all(|quad| quad.height <= layout().cell.height)
    );
}

#[test]
fn a_block_cursor_lets_the_glyph_fall_back_to_the_background() {
    let frame = drawn_with_caret("x", 0);
    let ink = frame
        .lines
        .iter()
        .flat_map(|line| &line.spans)
        .find(|span| span.text == "x")
        .map(|span| span.color);
    let background = Theme::default().background;
    assert_eq!(ink, Some((background.r, background.g, background.b)));
}

#[test]
fn bold_italic_takes_the_chosen_weight_and_style() {
    let face = Face {
        family: "Iosevka".to_string(),
        weight: 500,
        bold: 800,
        bold_italic: 900,
        oblique: true,
        ligatures: true,
        features: Vec::new(),
    };
    let attrs = attrs(&face, true, true);
    assert_eq!(attrs.weight, Weight(900));
    assert_eq!(attrs.style, Style::Oblique);
    assert!(attrs.font_features.features.is_empty());
}

#[test]
fn plain_text_takes_the_regular_weight() {
    let face = Face {
        family: "Iosevka".to_string(),
        weight: 500,
        bold: 800,
        bold_italic: 900,
        oblique: false,
        ligatures: true,
        features: Vec::new(),
    };
    assert_eq!(attrs(&face, false, false).weight, Weight(500));
    assert_eq!(attrs(&face, true, false).weight, Weight(800));
    assert_eq!(attrs(&face, false, true).style, Style::Italic);
}

#[test]
fn disabling_ligatures_and_custom_features_reach_attrs() {
    use glyphon::cosmic_text::FeatureTag;

    let face = Face {
        family: "Iosevka".to_string(),
        weight: 500,
        bold: 800,
        bold_italic: 900,
        oblique: false,
        ligatures: false,
        features: vec![(*b"zero", 1), (*b"ss01", 2)],
    };
    let a = attrs(&face, false, false);
    let pairs: Vec<(FeatureTag, u32)> = a
        .font_features
        .features
        .iter()
        .map(|f| (f.tag, f.value))
        .collect();
    assert!(pairs.contains(&(FeatureTag::new(b"calt"), 0)));
    assert!(pairs.contains(&(FeatureTag::new(b"liga"), 0)));
    assert!(pairs.contains(&(FeatureTag::new(b"clig"), 0)));
    assert!(pairs.contains(&(FeatureTag::new(b"dlig"), 0)));
    assert!(pairs.contains(&(FeatureTag::new(b"zero"), 1)));
    assert!(pairs.contains(&(FeatureTag::new(b"ss01"), 2)));
}

#[test]
fn a_rung_tab_shows_a_mark() {
    let tab = |rang| TabView {
        title: "build".to_string(),
        activity: Activity::Idle,
        confirming: false,
        rang,
    };
    let shown = |rang| {
        let bar = koral::tabs::layout(2, 800.0, 38.0);
        let views = [tab(false), tab(rang)];
        let chrome = Chrome {
            tabs: &views,
            active: 0,
            hovered: None,
            hovered_close: false,
            hovered_new: false,
            hovered_settings: false,
            settings_open: false,
            bar: &bar,
        };
        let mut frame = Frame::default();
        draw_chrome(&mut frame, &chrome, layout(), &Theme::default());
        frame
            .lines
            .iter()
            .flat_map(|line| &line.spans)
            .any(|span| span.text == "!")
    };
    assert!(shown(true));
    assert!(!shown(false));
}

#[test]
fn a_narrow_window_still_draws_the_search_bar() {
    let narrow = koral::frame::Layout {
        width: 60.0,
        ..layout()
    };
    let mut frame = Frame::default();
    koral::frame::draw_search_bar(&mut frame, "needle", false, narrow, &Theme::default());
    assert!(!frame.quads.is_empty());
}

#[test]
fn a_tab_kept_in_a_cell_draws_one_cell_wide() {
    assert_eq!(koral::frame::shown('\t'), ' ');
    assert_eq!(koral::frame::shown('\0'), ' ');
    assert_eq!(koral::frame::shown('\u{10EEEE}'), ' ');
    assert_eq!(koral::frame::shown('a'), 'a');
}

fn pieces(text: &str) -> Vec<(usize, String)> {
    let (x, width) = (layout().grid_x(), layout().cell.width);
    let line = |line: &koral::render::TextLine| {
        let text = line.spans.iter().map(|span| span.text.as_str()).collect();
        (((line.x - x) / width).round() as usize, text)
    };
    drawn(text).lines.iter().map(line).collect()
}

#[test]
fn text_after_a_wide_glyph_stays_on_its_column() {
    assert_eq!(
        pieces("a😀b"),
        [
            (0, "a".to_string()),
            (1, "😀".to_string()),
            (3, "b".to_string())
        ]
    );
}

#[test]
fn every_symbol_starts_on_its_own_column() {
    let expected =
        [(0, "日"), (2, "本"), (4, "❯"), (5, "x")].map(|(column, text)| (column, text.to_string()));
    assert_eq!(pieces("日本❯x"), expected);
}

#[test]
fn a_mark_stays_with_the_symbol_it_combines_with() {
    assert_eq!(
        pieces("❯\u{301}x"),
        [(0, "❯\u{301}".to_string()), (1, "x".to_string())]
    );
}

#[test]
fn plain_and_accented_text_stay_in_one_piece() {
    assert_eq!(pieces("ação  ok"), [(0, "ação  ok".to_string())]);
}

#[test]
fn spaces_after_a_symbol_draw_nothing() {
    assert_eq!(pieces("❯   "), [(0, "❯".to_string())]);
}

/// The shape closure of these tests: the entry is the key itself.
fn shape_into(cache: &mut LineCache<u64>, keys: &[u64]) -> usize {
    cache.prepare(keys, |key, _| *key)
}

/// A TUI grows the cache every frame it scrolls. Passing the cap must cost the oldest lines, not
/// the whole cache: emptying it is what re-shaped a screen on a tab switch.
#[test]
fn a_full_cache_loses_its_oldest_lines_instead_of_going_wholesale() {
    let mut cache: LineCache<u64> = LineCache::default();
    let first: Vec<u64> = (0..MAX_CACHED_LINES as u64).collect();
    assert_eq!(shape_into(&mut cache, &first), first.len());

    let next: Vec<u64> = (1_000_000..1_000_100).collect();
    assert_eq!(shape_into(&mut cache, &next), next.len());

    assert!(
        cache.get(&(MAX_CACHED_LINES as u64 - 1)).is_some(),
        "the remains of the previous screen are still shaped"
    );
    assert!(
        cache.get(&0).is_none(),
        "the oldest lines made room for the new ones"
    );
    assert!(
        cache.len() <= MAX_CACHED_LINES,
        "the cache stays under the cap"
    );
}

#[test]
fn a_repeated_key_is_shaped_once() {
    let mut cache: LineCache<u64> = LineCache::default();
    assert_eq!(
        shape_into(&mut cache, &[7, 7, 7]),
        1,
        "a key repeating in a frame is one line"
    );
    assert_eq!(
        shape_into(&mut cache, &[7, 7, 7]),
        0,
        "a cached line is not shaped again"
    );
    assert_eq!(cache.len(), 1);
}

/// The shape closure receives the line's place in the frame, not a count of earlier misses. A hit
/// or a duplicate must not shift the index of the misses that follow it.
#[test]
fn mixed_hits_and_duplicates_keep_miss_indices_aligned_to_the_frame() {
    let mut cache = LineCache::default();
    shape_into(&mut cache, &[10, 30]);

    let keys = [10, 20, 10, 30, 40, 20];
    let mut shaped_at = Vec::new();
    let shaped = cache.prepare(&keys, |key, index| {
        shaped_at.push((*key, index));
        *key
    });

    assert_eq!(shaped, 2, "20 and 40 are the only misses");
    assert_eq!(
        shaped_at,
        [(20, 1), (40, 4)],
        "hits and duplicates do not shift the index of a later miss"
    );
}

#[test]
fn the_frame_being_drawn_is_never_evicted() {
    let mut cache: LineCache<u64> = LineCache::default();
    let frame: Vec<u64> = (0..MAX_CACHED_LINES as u64 * 2).collect();
    assert_eq!(shape_into(&mut cache, &frame), frame.len());
    assert!(
        frame.iter().all(|key| cache.get(key).is_some()),
        "a frame larger than the cap keeps every line of itself"
    );
    assert_eq!(cache.len(), frame.len(), "and holds nothing else");
}

#[test]
fn the_cache_stays_under_the_cap_across_screens() {
    let mut cache: LineCache<u64> = LineCache::default();
    for screen in 0..200u64 {
        let keys: Vec<u64> = (0..50).map(|key| screen * 50 + key).collect();
        shape_into(&mut cache, &keys);
    }
    assert!(
        cache.len() <= MAX_CACHED_LINES,
        "200 screens do not outgrow the cap: {}",
        cache.len()
    );
}

#[test]
fn eviction_leaves_the_screen_it_just_drew() {
    let mut cache: LineCache<u64> = LineCache::default();
    let screen: Vec<u64> = (0..MAX_CACHED_LINES as u64 + 200).collect();
    assert_eq!(shape_into(&mut cache, &screen), screen.len());
    assert_eq!(
        shape_into(&mut cache, &screen),
        0,
        "the screen that triggered an eviction is shaped nothing"
    );
}

#[test]
fn search_bar_shows_match_counter_when_present() {
    let mut frame = Frame::default();
    koral::frame::draw_search_bar_status(
        &mut frame,
        "needle",
        true,
        Some((3, 14)),
        layout(),
        &Theme::default(),
    );
    let text: String = frame
        .lines
        .iter()
        .flat_map(|line| &line.spans)
        .map(|span| span.text.as_str())
        .collect();
    assert!(text.contains("3/14"), "expected 3/14 in {text:?}");
}

#[test]
fn scrollback_shows_thumb_and_failed_block_ticks() {
    let term = Term::new(Config::default(), &TermSize::new(20, 5), VoidListener);
    let mut session = Session::new(term);
    let now = Instant::now();
    for i in 0..10 {
        let status = if i == 3 { 1 } else { 0 };
        let seq = format!(
            "\x1b]133;A\x1b\\$ \x1b]133;B\x1b\\cmd{i}\r\n\x1b]133;C\x1b\\out{i}\r\n\x1b]133;D;{status}\x1b\\"
        );
        session.feed(seq.as_bytes(), now);
    }
    let rows = rows(&session.term, &session.blocks);
    let theme = Theme::default();
    let cursor = CursorView {
        focused: true,
        visible: true,
    };
    let view = koral::frame::Layout {
        height: 38.0 + 5.0 * 20.0,
        ..layout()
    };

    let mut at_bottom = Frame::default();
    draw_terminal(
        &mut at_bottom,
        &session.term,
        &session.blocks,
        &rows,
        0,
        cursor,
        view,
        &theme,
    );
    assert!(
        !at_bottom
            .quads
            .iter()
            .any(|q| q.color == rgba(theme.muted, 0.45))
    );

    let mut scrolled = Frame::default();
    draw_terminal(
        &mut scrolled,
        &session.term,
        &session.blocks,
        &rows,
        3,
        cursor,
        view,
        &theme,
    );
    assert!(
        scrolled
            .quads
            .iter()
            .any(|q| q.color == rgba(theme.muted, 0.45)),
        "expected scroll thumb"
    );
    assert!(
        scrolled
            .quads
            .iter()
            .any(|q| q.color == rgba(theme.failure, 0.85)),
        "expected failure tick"
    );
}

#[test]
fn a_glyph_missing_from_the_primary_face_splits_so_following_text_stays_aligned() {
    let term = Term::new(Config::default(), &TermSize::new(20, 5), VoidListener);
    let mut session = Session::new(term);
    session.feed("ação ok".as_bytes(), Instant::now());
    let rows = rows(&session.term, &session.blocks);
    let mut frame = Frame::default();
    let cursor = CursorView {
        focused: true,
        visible: true,
    };
    let pane = layout().pane();
    draw_terminal_in(
        &mut frame,
        &session.term,
        &session.blocks,
        &rows,
        0,
        cursor,
        pane,
        Coverage::default().without('ç'),
        &Theme::default(),
    );
    let x0 = pane.grid_x();
    let cell_w = pane.cell.width;
    let parts: Vec<(usize, String)> = frame
        .lines
        .iter()
        .map(|line| {
            let text: String = line.spans.iter().map(|s| s.text.as_str()).collect();
            (((line.x - x0) / cell_w).round() as usize, text)
        })
        .collect();
    assert_eq!(
        parts,
        [
            (0, "a".to_string()),
            (1, "ç".to_string()),
            (2, "ão ok".to_string())
        ]
    );
}

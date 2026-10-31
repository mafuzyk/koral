use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};
use koral::theme::{Theme, hex};

const fn rgb(r: u8, g: u8, b: u8) -> Rgb {
    Rgb { r, g, b }
}

fn resolve(color: Color) -> Rgb {
    Theme::default().resolve(color, &Colors::default())
}

#[test]
fn hex_parses_sharky_colours() {
    assert_eq!(hex("#071923"), Some(rgb(0x07, 0x19, 0x23)));
}

#[test]
fn hex_rejects_anything_else() {
    assert_eq!(hex("071923"), None);
    assert_eq!(hex("#07192"), None);
    assert_eq!(hex("#07192g"), None);
    assert_eq!(hex("#é7192"), None);
}

#[test]
fn explicit_colours_pass_through() {
    assert_eq!(resolve(Color::Spec(rgb(1, 2, 3))), rgb(1, 2, 3));
}

#[test]
fn named_colours_come_from_the_theme() {
    let theme = Theme::default();
    assert_eq!(resolve(Color::Named(NamedColor::Background)), theme.background);
    assert_eq!(resolve(Color::Named(NamedColor::Foreground)), theme.foreground);
    assert_eq!(resolve(Color::Named(NamedColor::Red)), theme.ansi[1]);
    assert_eq!(resolve(Color::Named(NamedColor::BrightCyan)), theme.ansi[14]);
}

#[test]
fn indexed_colours_cover_the_256_palette() {
    let theme = Theme::default();
    assert_eq!(resolve(Color::Indexed(3)), theme.ansi[3]);
    assert_eq!(resolve(Color::Indexed(16)), rgb(0, 0, 0));
    assert_eq!(resolve(Color::Indexed(52)), rgb(95, 0, 0));
    assert_eq!(resolve(Color::Indexed(231)), rgb(255, 255, 255));
    assert_eq!(resolve(Color::Indexed(232)), rgb(8, 8, 8));
    assert_eq!(resolve(Color::Indexed(255)), rgb(238, 238, 238));
}

#[test]
fn dim_colours_are_darker_than_their_base() {
    let theme = Theme::default();
    let dim = resolve(Color::Named(NamedColor::DimRed));
    assert!(dim.r < theme.ansi[1].r);
}

#[test]
fn colours_set_by_the_running_app_win() {
    let mut overrides = Colors::default();
    overrides[1] = Some(rgb(9, 9, 9));
    let theme = Theme::default();
    assert_eq!(theme.resolve(Color::Indexed(1), &overrides), rgb(9, 9, 9));
}

#[test]
fn the_palette_keeps_text_readable_on_the_background() {
    let theme = Theme::default();
    let luminance = |c: Rgb| {
        let channel = |v: u8| {
            let v = f32::from(v) / 255.0;
            if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b)
    };
    let contrast = (luminance(theme.foreground) + 0.05) / (luminance(theme.background) + 0.05);
    assert!(contrast > 7.0, "{contrast}");
}

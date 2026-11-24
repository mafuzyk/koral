use koral::config::{Config, parse, update};
use koral::input::{Action, binding};
use koral::settings::{
    Command, FIELDS, Outcome, Picker, PickerKind, Settings, SettingsView, Zone, tab_zone, value,
    visible,
};
use koral::tabs::{Hit, hit, layout};
use winit::keyboard::{Key, ModifiersState};

fn index(key: &str) -> usize {
    FIELDS.iter().position(|field| field.key == key).unwrap()
}

fn on(key: &str) -> Settings {
    Settings {
        selected: index(key),
        ..Settings::default()
    }
}

fn drawn(settings: &Settings, config: &Config, error: Option<&str>) -> koral::frame::Frame {
    let cell = koral::render::Cell {
        width: 8.0,
        height: 20.0,
    };
    let layout = koral::frame::Layout {
        width: 1000.0,
        height: 700.0,
        scale: 1.0,
        padding: 8.0,
        cell,
    };
    let mut frame = koral::frame::Frame::default();
    koral::frame::draw_settings(
        &mut frame,
        settings,
        config,
        error,
        layout,
        &koral::theme::Theme::default(),
    );
    frame
}

fn text(frame: &koral::frame::Frame) -> Vec<String> {
    frame
        .lines
        .iter()
        .flat_map(|line| &line.spans)
        .map(|span| span.text.clone())
        .collect()
}

fn fonts() -> Vec<String> {
    [
        "Fira Code",
        "Fira Mono",
        "IosevkaTerm Nerd Font Mono",
        "Monocraft",
    ]
    .map(String::from)
    .to_vec()
}

fn picking_font() -> Settings {
    let mut settings = Settings {
        fonts: fonts(),
        ..on("font")
    };
    settings.handle(Command::Confirm, &Config::default());
    settings
}

fn picking_theme() -> Settings {
    let mut settings = on("theme");
    settings.handle(Command::Confirm, &Config::default());
    settings
}

#[test]
fn the_gear_sits_at_the_right_end_of_the_strip() {
    let bar = layout(2, 1000.0, 36.0);
    assert_eq!(bar.settings.x + bar.settings.width, 1000.0);
    assert_eq!(hit(&bar, 990.0, 10.0), Hit::Settings);
}

#[test]
fn tabs_never_run_under_the_gear() {
    let bar = layout(30, 600.0, 36.0);
    assert!(bar.new_tab.x + bar.new_tab.width <= bar.settings.x);
}

#[test]
fn a_shortcut_opens_settings() {
    let mods = ModifiersState::CONTROL | ModifiersState::SHIFT;
    assert_eq!(
        binding(&Key::Character("<".into()), mods),
        Some(Action::OpenSettings)
    );
    assert_eq!(
        binding(&Key::Character(",".into()), mods),
        Some(Action::OpenSettings)
    );
}

#[test]
fn up_and_down_wrap_around() {
    let mut settings = Settings::default();
    settings.handle(Command::Up, &Config::default());
    assert_eq!(settings.selected, FIELDS.len() - 1);
    settings.handle(Command::Down, &Config::default());
    assert_eq!(settings.selected, 0);
}

#[test]
fn more_raises_a_number_by_its_step() {
    let outcome = on("font_size").handle(Command::More, &Config::default());
    assert_eq!(outcome, Outcome::Change("font_size", "14".to_string()));
}

#[test]
fn less_lowers_opacity_without_float_noise() {
    let outcome = on("opacity").handle(Command::Less, &Config::default());
    assert_eq!(outcome, Outcome::Change("opacity", "0.75".to_string()));
}

#[test]
fn numbers_stop_at_their_limits() {
    let config = Config {
        opacity: 1.0,
        ..Config::default()
    };
    assert_eq!(
        on("opacity").handle(Command::More, &config),
        Outcome::Nothing
    );
}

#[test]
fn less_and_more_do_nothing_on_text_fields() {
    assert_eq!(
        on("shell").handle(Command::More, &Config::default()),
        Outcome::Nothing
    );
}

#[test]
fn confirm_starts_editing_text_with_the_current_value() {
    let mut settings = on("shell");
    let config = Config {
        shell: Some("fish".to_string()),
        ..Config::default()
    };
    settings.handle(Command::Confirm, &config);
    assert_eq!(settings.editing.as_deref(), Some("fish"));
}

#[test]
fn typing_edits_and_confirm_saves() {
    let mut settings = on("shell");
    let config = Config::default();
    settings.handle(Command::Confirm, &config);
    settings.handle(Command::Type('f'), &config);
    settings.handle(Command::Type('i'), &config);
    settings.handle(Command::Type('x'), &config);
    settings.handle(Command::Erase, &config);
    assert_eq!(
        settings.handle(Command::Confirm, &config),
        Outcome::Change("shell", "fi".to_string())
    );
    assert_eq!(settings.editing, None);
}

#[test]
fn cancel_leaves_editing_before_it_closes() {
    let mut settings = on("shell");
    let config = Config::default();
    settings.handle(Command::Confirm, &config);
    assert_eq!(settings.handle(Command::Cancel, &config), Outcome::Nothing);
    assert_eq!(settings.editing, None);
    assert_eq!(settings.handle(Command::Cancel, &config), Outcome::Close);
}

#[test]
fn values_read_like_the_file() {
    let config = Config::default();
    assert_eq!(value(index("font_size"), &config), "13.5");
    assert_eq!(value(index("opacity"), &config), "0.8");
    assert_eq!(value(index("scrollback"), &config), "10000");
    assert_eq!(value(index("shell"), &config), "");
}

#[test]
fn update_returns_the_new_text_and_config() {
    let (source, config) = update("// mine\n", "opacity", "0.6").unwrap();
    assert_eq!(source, "// mine\npolyp opacity 0.6\n");
    assert_eq!(config.opacity, 0.6);
}

#[test]
fn update_refuses_a_value_the_parser_would_reject() {
    assert!(update("", "font", "").is_err());
    assert!(update("polyp opacity 0.5\n", "opacity", "9").is_err());
    assert!(update("", "theme", "not-a-theme").is_err());
}

#[test]
fn update_ignores_errors_that_were_already_in_the_file() {
    let source = "whale oops\n";
    assert_eq!(parse(source).1.len(), 1);
    assert!(update(source, "opacity", "0.6").is_ok());
}

#[test]
fn choice_field_cycles_oblique_and_italic() {
    assert_eq!(
        on("italic_style").handle(Command::Confirm, &Config::default()),
        Outcome::Change("italic_style", "italic".to_string())
    );
    let italic_cfg = Config {
        oblique: false,
        ..Config::default()
    };
    assert_eq!(
        on("italic_style").handle(Command::Less, &italic_cfg),
        Outcome::Change("italic_style", "oblique".to_string())
    );
}

#[test]
fn theme_row_cycles_and_theme_picker_previews_and_applies() {
    let outcome = on("theme").handle(Command::More, &Config::default());
    assert_eq!(outcome, Outcome::Change("theme", "terminal".to_string()));

    let mut settings = picking_theme();
    assert_eq!(
        settings.picker.as_ref().map(|p| p.kind),
        Some(PickerKind::Theme)
    );
    assert_eq!(settings.preview_theme(), Some("sharky"));
    settings.handle(Command::Type('d'), &Config::default());
    settings.handle(Command::Type('r'), &Config::default());
    settings.handle(Command::Type('a'), &Config::default());
    settings.handle(Command::Type('c'), &Config::default());
    assert_eq!(settings.preview_theme(), Some("dracula"));
    assert_eq!(
        settings.handle(Command::Confirm, &Config::default()),
        Outcome::Change("theme", "dracula".to_string())
    );
    assert_eq!(settings.preview_theme(), None);
}

#[test]
fn tab_view_draws_sections_preview_and_hits_rows_and_arrows() {
    let frame = drawn(&Settings::default(), &Config::default(), None);
    let lines = text(&frame);
    assert!(lines.iter().any(|l| l == "Settings"));
    assert!(lines.iter().any(|l| l == "APPEARANCE"));
    assert!(lines.iter().any(|l| l == "ANSI 0–15"));

    let view = SettingsView::new(1000.0, 700.0, 38.0, 1.0, 20.0);
    assert!(view.right.is_some());
    let settings = Settings::default();
    // Row 0 is header ("APPEARANCE"), row 1 is "theme".
    let theme_y = view.left.y + 1.5 * view.row_height;
    assert_eq!(
        tab_zone(view, view.left.x + 20.0, theme_y, &settings),
        Zone::Row(0)
    );
    let (less_x, more_x) = view.arrows(FIELDS[0].kind).unwrap();
    assert_eq!(
        tab_zone(view, less_x + 4.0, theme_y, &settings),
        Zone::Less(0)
    );
    assert_eq!(
        tab_zone(view, more_x + 4.0, theme_y, &settings),
        Zone::More(0)
    );
}

#[test]
fn toggles_and_substitution_notice_render_on_the_tab() {
    assert!(
        text(&drawn(&on("matugen"), &Config::default(), None))
            .iter()
            .any(|line| line == "off")
    );
    let config = Config {
        matugen: true,
        ..Config::default()
    };
    assert!(
        text(&drawn(&on("matugen"), &config, None))
            .iter()
            .any(|line| line == "on")
    );

    let frame = drawn(
        &Settings::default(),
        &Config::default(),
        Some("Can't use Missing. Showing Iosevka."),
    );
    let lines = text(&frame);
    assert!(lines.iter().any(|line| line == "Can't use Missing."));
    assert!(!lines.iter().any(|line| line.contains("Showing")));

    assert_eq!(
        on("clipboard_read").handle(Command::Confirm, &Config::default()),
        Outcome::Change("clipboard_read", "on".to_string())
    );
}

#[test]
fn font_picker_filters_wraps_and_picks_rows() {
    let settings = picking_font();
    assert_eq!(
        settings.picker,
        Some(Picker {
            kind: PickerKind::Font,
            query: String::new(),
            selected: 2
        })
    );

    let mut s = picking_font();
    let cfg = Config::default();
    s.handle(Command::Down, &cfg);
    s.handle(Command::Down, &cfg);
    assert_eq!(s.picker.as_ref().unwrap().selected, 0);
    s.handle(Command::Up, &cfg);
    assert_eq!(s.picker.as_ref().unwrap().selected, 3);

    s.handle(Command::Type('F'), &cfg);
    s.handle(Command::Type('i'), &cfg);
    assert_eq!(s.found(), ["Fira Code", "Fira Mono"]);
    assert_eq!(
        s.handle(Command::Confirm, &cfg),
        Outcome::Change("font", "Fira Code".to_string())
    );
    assert_eq!(s.picker, None);
    assert_eq!(visible(50, 100, 6), 47..53);

    let fonts_many: Vec<String> = (0..20).map(|number| format!("Font {number}")).collect();
    let mut s2 = Settings {
        fonts: fonts_many,
        picker: Some(Picker {
            kind: PickerKind::Font,
            query: String::new(),
            selected: 10,
        }),
        ..on("font")
    };
    assert!(s2.pick(0, 6));
    assert_eq!(s2.picker.unwrap().selected, 7);
}

use koral::config::{Config, parse, update};
use koral::input::{Action, binding};
use koral::settings::{Command, FIELDS, Outcome, Panel, Settings, Zone, value, zone};
use koral::tabs::{Hit, hit, layout};
use winit::keyboard::{Key, ModifiersState};

fn index(key: &str) -> usize {
    FIELDS.iter().position(|field| field.key == key).unwrap()
}

fn on(key: &str) -> Settings {
    Settings { selected: index(key), editing: None }
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
fn a_shortcut_opens_the_panel() {
    let mods = ModifiersState::CONTROL | ModifiersState::SHIFT;
    assert_eq!(binding(&Key::Character("<".into()), mods), Some(Action::OpenSettings));
    assert_eq!(binding(&Key::Character(",".into()), mods), Some(Action::OpenSettings));
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
    let config = Config { opacity: 1.0, ..Config::default() };
    assert_eq!(on("opacity").handle(Command::More, &config), Outcome::Nothing);
}

#[test]
fn less_and_more_do_nothing_on_text_fields() {
    assert_eq!(on("font").handle(Command::More, &Config::default()), Outcome::Nothing);
}

#[test]
fn confirm_starts_editing_text_with_the_current_value() {
    let mut settings = on("font");
    settings.handle(Command::Confirm, &Config::default());
    assert_eq!(settings.editing.as_deref(), Some("IosevkaTerm Nerd Font Mono"));
}

#[test]
fn confirm_does_not_edit_a_number() {
    let mut settings = on("opacity");
    settings.handle(Command::Confirm, &Config::default());
    assert_eq!(settings.editing, None);
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
    assert_eq!(settings.handle(Command::Confirm, &config), Outcome::Change("shell", "fi".to_string()));
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
fn up_and_down_are_ignored_while_editing() {
    let mut settings = on("shell");
    let config = Config::default();
    settings.handle(Command::Confirm, &config);
    settings.handle(Command::Down, &config);
    assert_eq!(settings.selected, index("shell"));
}

#[test]
fn values_read_like_the_file() {
    let config = Config::default();
    assert_eq!(value(index("font_size"), &config), "13.5");
    assert_eq!(value(index("opacity"), &config), "0.8");
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
}

#[test]
fn update_ignores_errors_that_were_already_in_the_file() {
    let source = "whale oops\n";
    assert_eq!(parse(source).1.len(), 1);
    assert!(update(source, "opacity", "0.6").is_ok());
}

const PANEL: Panel = Panel { x: 100.0, y: 50.0, width: 400.0, row_height: 30.0 };

#[test]
fn clicks_find_their_row() {
    assert_eq!(zone(PANEL, 150.0, 55.0), Zone::Row(0));
    assert_eq!(zone(PANEL, 150.0, 85.0), Zone::Row(1));
}

#[test]
fn clicks_on_the_arrows_of_a_number_row_step_it() {
    let row = index("font_size");
    let y = PANEL.y + row as f32 * PANEL.row_height + 5.0;
    assert_eq!(zone(PANEL, 480.0, y), Zone::More(row));
    assert_eq!(zone(PANEL, 380.0, y), Zone::Less(row));
}

#[test]
fn text_rows_have_no_arrows() {
    let row = index("font");
    let y = PANEL.y + row as f32 * PANEL.row_height + 5.0;
    assert_eq!(zone(PANEL, 480.0, y), Zone::Row(row));
}

#[test]
fn clicks_below_the_rows_stay_inside_the_panel() {
    let y = PANEL.y + FIELDS.len() as f32 * PANEL.row_height + 5.0;
    assert_eq!(zone(PANEL, 150.0, y), Zone::Inside);
}

#[test]
fn clicks_away_from_the_panel_are_outside() {
    assert_eq!(zone(PANEL, 50.0, 55.0), Zone::Outside);
}

#[test]
fn a_toggle_flips_with_enter_and_either_arrow() {
    for command in [Command::Confirm, Command::Less, Command::More] {
        let outcome = on("matugen").handle(command, &Config::default());
        assert_eq!(outcome, Outcome::Change("matugen", "on".to_string()));
    }
}

#[test]
fn a_toggle_that_is_on_turns_off() {
    let config = Config { matugen: true, ..Config::default() };
    let outcome = on("matugen").handle(Command::Confirm, &config);
    assert_eq!(outcome, Outcome::Change("matugen", "off".to_string()));
}

#[test]
fn a_toggle_opens_no_text_editor() {
    let mut settings = on("matugen");
    settings.handle(Command::Confirm, &Config::default());
    assert_eq!(settings.editing, None);
}

#[test]
fn a_toggle_row_applies_through_update() {
    let (source, config) = update("", "matugen", "on").unwrap();
    assert_eq!(source, "polyp matugen on\n");
    assert!(config.matugen);
}

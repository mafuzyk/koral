use koral::config::{Config, parse, update};
use koral::input::{Action, binding};
use koral::settings::{Command, FIELDS, Outcome, Panel, Picker, Settings, Zone, value, visible, zone};
use koral::tabs::{Hit, hit, layout};
use winit::keyboard::{Key, ModifiersState};

fn index(key: &str) -> usize {
    FIELDS.iter().position(|field| field.key == key).unwrap()
}

fn on(key: &str) -> Settings {
    Settings { selected: index(key), ..Settings::default() }
}

fn fonts() -> Vec<String> {
    ["Fira Code", "Fira Mono", "IosevkaTerm Nerd Font Mono", "Monocraft"].map(String::from).to_vec()
}

fn picking() -> Settings {
    let mut settings = Settings { fonts: fonts(), ..on("font") };
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
    assert_eq!(on("shell").handle(Command::More, &Config::default()), Outcome::Nothing);
}

#[test]
fn confirm_starts_editing_text_with_the_current_value() {
    let mut settings = on("shell");
    let config = Config { shell: Some("fish".to_string()), ..Config::default() };
    settings.handle(Command::Confirm, &config);
    assert_eq!(settings.editing.as_deref(), Some("fish"));
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

#[test]
fn confirm_on_the_font_opens_the_list_at_the_current_font() {
    let settings = picking();
    assert_eq!(settings.editing, None);
    assert_eq!(settings.picker, Some(Picker { query: String::new(), selected: 2 }));
}

#[test]
fn a_font_that_is_not_listed_opens_the_list_at_the_top() {
    let mut settings = Settings { fonts: fonts(), ..on("font") };
    let config = Config { font: "Missing".to_string(), ..Config::default() };
    settings.handle(Command::Confirm, &config);
    assert_eq!(settings.picker.unwrap().selected, 0);
}

#[test]
fn the_list_moves_and_wraps_around() {
    let mut settings = picking();
    let config = Config::default();
    settings.handle(Command::Down, &config);
    settings.handle(Command::Down, &config);
    assert_eq!(settings.picker.as_ref().unwrap().selected, 0);
    settings.handle(Command::Up, &config);
    assert_eq!(settings.picker.as_ref().unwrap().selected, 3);
}

#[test]
fn confirm_applies_the_chosen_font_and_closes_the_list() {
    let mut settings = picking();
    let config = Config::default();
    settings.handle(Command::Down, &config);
    assert_eq!(settings.handle(Command::Confirm, &config), Outcome::Change("font", "Monocraft".to_string()));
    assert_eq!(settings.picker, None);
}

#[test]
fn typing_filters_the_list_whatever_the_case() {
    let mut settings = picking();
    let config = Config::default();
    settings.handle(Command::Type('F'), &config);
    settings.handle(Command::Type('i'), &config);
    assert_eq!(settings.found(), ["Fira Code", "Fira Mono"]);
    assert_eq!(settings.picker.as_ref().unwrap().selected, 0);
    settings.handle(Command::Erase, &config);
    settings.handle(Command::Erase, &config);
    assert_eq!(settings.found().len(), 4);
}

#[test]
fn confirm_with_no_match_keeps_the_list_open() {
    let mut settings = picking();
    let config = Config::default();
    settings.handle(Command::Type('z'), &config);
    assert_eq!(settings.handle(Command::Confirm, &config), Outcome::Nothing);
    assert!(settings.picker.is_some());
}

#[test]
fn moving_in_an_empty_list_does_nothing() {
    let mut settings = on("font");
    let config = Config::default();
    settings.handle(Command::Confirm, &config);
    settings.handle(Command::Down, &config);
    settings.handle(Command::Up, &config);
    assert_eq!(settings.picker.unwrap().selected, 0);
}

#[test]
fn cancel_leaves_the_list_before_it_closes() {
    let mut settings = picking();
    let config = Config::default();
    assert_eq!(settings.handle(Command::Cancel, &config), Outcome::Nothing);
    assert_eq!(settings.picker, None);
    assert_eq!(settings.handle(Command::Cancel, &config), Outcome::Close);
}

#[test]
fn the_visible_part_of_the_list_follows_the_selection() {
    assert_eq!(visible(0, 100), 0..6);
    assert_eq!(visible(50, 100), 47..53);
    assert_eq!(visible(99, 100), 94..100);
    assert_eq!(visible(1, 3), 0..3);
}

#[test]
fn a_click_selects_the_font_on_its_row() {
    let mut settings = picking();
    assert!(settings.pick(3));
    assert_eq!(settings.picker.unwrap().selected, 3);
}

#[test]
fn a_click_below_the_last_font_selects_nothing() {
    let mut settings = picking();
    assert!(!settings.pick(4));
    assert_eq!(settings.picker.unwrap().selected, 2);
}

#[test]
fn a_click_counts_from_the_top_of_the_scrolled_list() {
    let fonts = (0..20).map(|number| format!("Font {number}")).collect();
    let mut settings = Settings { fonts, picker: Some(Picker { query: String::new(), selected: 10 }), ..on("font") };
    assert!(settings.pick(0));
    assert_eq!(settings.picker.unwrap().selected, 7);
}

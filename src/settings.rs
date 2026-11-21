use crate::config::Config;
use crate::tabs::Rect;
use crate::theme::{BUILTINS, ThemeInfo, theme_name};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Appearance,
    Typography,
    Terminal,
}

impl Section {
    pub const ALL: [Self; 3] = [Self::Appearance, Self::Typography, Self::Terminal];

    pub fn label(self) -> &'static str {
        match self {
            Self::Appearance => "APPEARANCE",
            Self::Typography => "TYPOGRAPHY",
            Self::Terminal => "TERMINAL",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Theme,
    Font,
    Text,
    Number { min: f32, max: f32, step: f32 },
    Choice(&'static [&'static str]),
    Toggle,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Field {
    pub section: Section,
    pub key: &'static str,
    pub label: &'static str,
    pub kind: Kind,
}

const ITALIC_CHOICES: &[&str] = &["oblique", "italic"];

pub const FIELDS: [Field; 12] = [
    Field { section: Section::Appearance, key: "theme", label: "Theme", kind: Kind::Theme },
    Field { section: Section::Appearance, key: "matugen", label: "Colours from matugen", kind: Kind::Toggle },
    Field {
        section: Section::Appearance,
        key: "opacity",
        label: "Opacity",
        kind: Kind::Number { min: 0.1, max: 1.0, step: 0.05 },
    },
    Field {
        section: Section::Appearance,
        key: "padding",
        label: "Padding",
        kind: Kind::Number { min: 0.0, max: 64.0, step: 2.0 },
    },
    Field { section: Section::Typography, key: "font", label: "Font", kind: Kind::Font },
    Field {
        section: Section::Typography,
        key: "font_size",
        label: "Font size",
        kind: Kind::Number { min: 6.0, max: 72.0, step: 0.5 },
    },
    Field {
        section: Section::Typography,
        key: "font_weight",
        label: "Regular weight",
        kind: Kind::Number { min: 100.0, max: 900.0, step: 100.0 },
    },
    Field {
        section: Section::Typography,
        key: "bold_weight",
        label: "Bold weight",
        kind: Kind::Number { min: 100.0, max: 900.0, step: 100.0 },
    },
    Field {
        section: Section::Typography,
        key: "italic_style",
        label: "Italic style",
        kind: Kind::Choice(ITALIC_CHOICES),
    },
    Field { section: Section::Terminal, key: "shell", label: "Shell for new tabs", kind: Kind::Text },
    Field {
        section: Section::Terminal,
        key: "scrollback",
        label: "Scrollback lines",
        kind: Kind::Number { min: 0.0, max: 1_000_000.0, step: 1000.0 },
    },
    Field { section: Section::Terminal, key: "clipboard_read", label: "Clipboard read (OSC 52)", kind: Kind::Toggle },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormRow {
    Header(Section),
    Field(usize),
}

pub fn form_rows() -> Vec<FormRow> {
    let mut rows = Vec::with_capacity(FIELDS.len() + Section::ALL.len());
    let mut last = None;
    for (index, field) in FIELDS.iter().enumerate() {
        if last != Some(field.section) {
            rows.push(FormRow::Header(field.section));
            last = Some(field.section);
        }
        rows.push(FormRow::Field(index));
    }
    rows
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Up,
    Down,
    Less,
    More,
    Confirm,
    Cancel,
    Erase,
    Type(char),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Nothing,
    Close,
    Change(&'static str, String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerKind {
    Font,
    Theme,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Picker {
    pub kind: PickerKind,
    pub query: String,
    pub selected: usize,
}

impl Default for Picker {
    fn default() -> Self {
        Self { kind: PickerKind::Font, query: String::new(), selected: 0 }
    }
}

#[derive(Debug, Default)]
pub struct Settings {
    pub selected: usize,
    pub scroll: usize,
    pub editing: Option<String>,
    pub picker: Option<Picker>,
    pub fonts: Vec<String>,
}

impl Settings {
    pub fn new(fonts: Vec<String>) -> Self {
        Self { fonts, ..Self::default() }
    }

    pub fn preview_theme(&self) -> Option<&'static str> {
        let picker = self.picker.as_ref().filter(|p| p.kind == PickerKind::Theme)?;
        self.found_themes().get(picker.selected).map(|info| info.id)
    }

    pub fn handle(&mut self, command: Command, config: &Config) -> Outcome {
        if self.editing.is_some() {
            return self.handle_editing(command);
        }
        if self.picker.is_some() {
            return self.handle_picker(command);
        }
        let kind = FIELDS[self.selected].kind;
        match kind {
            Kind::Toggle => match command {
                Command::Less | Command::More | Command::Confirm => return flip(self.selected, config),
                _ => {}
            },
            Kind::Choice(choices) => match command {
                Command::Less => return cycle_choice(self.selected, -1, choices, config),
                Command::More | Command::Confirm => return cycle_choice(self.selected, 1, choices, config),
                _ => {}
            },
            Kind::Theme => match command {
                Command::Less => return cycle_theme(-1, config),
                Command::More => return cycle_theme(1, config),
                Command::Confirm => {
                    let current = config.theme.as_deref().unwrap_or("sharky");
                    let selected = BUILTINS.iter().position(|info| info.id == current).unwrap_or(0);
                    self.picker = Some(Picker { kind: PickerKind::Theme, query: String::new(), selected });
                    return Outcome::Nothing;
                }
                _ => {}
            },
            _ => {}
        }
        match command {
            Command::Up => self.selected = (self.selected + FIELDS.len() - 1) % FIELDS.len(),
            Command::Down => self.selected = (self.selected + 1) % FIELDS.len(),
            Command::Less => return step(self.selected, -1.0, config),
            Command::More => return step(self.selected, 1.0, config),
            Command::Confirm if kind == Kind::Text => {
                self.editing = Some(value(self.selected, config));
            }
            Command::Confirm if kind == Kind::Font => {
                let selected = self.fonts.iter().position(|name| *name == config.font).unwrap_or(0);
                self.picker = Some(Picker { kind: PickerKind::Font, query: String::new(), selected });
            }
            Command::Cancel => return Outcome::Close,
            _ => {}
        }
        Outcome::Nothing
    }

    fn handle_editing(&mut self, command: Command) -> Outcome {
        match command {
            Command::Type(ch) => self.edit(|text| text.push(ch)),
            Command::Erase => self.edit(|text| {
                text.pop();
            }),
            Command::Cancel => self.editing = None,
            Command::Confirm => {
                if let Some(text) = self.editing.take() {
                    return Outcome::Change(FIELDS[self.selected].key, text);
                }
            }
            _ => {}
        }
        Outcome::Nothing
    }

    fn handle_picker(&mut self, command: Command) -> Outcome {
        let count = self.picker_count().max(1);
        let chosen = self.chosen_picker_value();
        let kind = self.picker.as_ref().map(|p| p.kind);
        let Some(picker) = &mut self.picker else {
            return Outcome::Nothing;
        };
        match command {
            Command::Up => picker.selected = (picker.selected + count - 1) % count,
            Command::Down => picker.selected = (picker.selected + 1) % count,
            Command::Type(ch) => {
                picker.query.push(ch);
                picker.selected = 0;
            }
            Command::Erase => {
                picker.query.pop();
                picker.selected = 0;
            }
            Command::Cancel => self.picker = None,
            Command::Confirm => {
                if let (Some(kind), Some(val)) = (kind, chosen) {
                    self.picker = None;
                    let key = match kind {
                        PickerKind::Font => "font",
                        PickerKind::Theme => "theme",
                    };
                    return Outcome::Change(key, val);
                }
            }
            _ => {}
        }
        Outcome::Nothing
    }

    /// Matching font names when the font picker is open.
    pub fn found(&self) -> Vec<&str> {
        let Some(picker) = self.picker.as_ref().filter(|p| p.kind == PickerKind::Font) else {
            return Vec::new();
        };
        let query = picker.query.to_lowercase();
        self.fonts.iter().map(String::as_str).filter(|name| name.to_lowercase().contains(&query)).collect()
    }

    /// Matching built-in themes when the theme picker is open.
    pub fn found_themes(&self) -> Vec<ThemeInfo> {
        let Some(picker) = self.picker.as_ref().filter(|p| p.kind == PickerKind::Theme) else {
            return Vec::new();
        };
        let query = picker.query.to_lowercase();
        BUILTINS
            .iter()
            .copied()
            .filter(|info| info.id.to_lowercase().contains(&query) || info.name.to_lowercase().contains(&query))
            .collect()
    }

    pub fn picker_count(&self) -> usize {
        match self.picker.as_ref().map(|p| p.kind) {
            Some(PickerKind::Font) => self.found().len(),
            Some(PickerKind::Theme) => self.found_themes().len(),
            None => 0,
        }
    }

    /// Selects the item drawn on a row of the active picker. False when empty.
    pub fn pick(&mut self, row: usize, capacity: usize) -> bool {
        let count = self.picker_count();
        let Some(picker) = &mut self.picker else {
            return false;
        };
        let index = visible(picker.selected, count, capacity).nth(row);
        picker.selected = index.unwrap_or(picker.selected);
        index.is_some()
    }

    fn chosen_picker_value(&self) -> Option<String> {
        let picker = self.picker.as_ref()?;
        match picker.kind {
            PickerKind::Font => self.found().get(picker.selected).map(|s| (*s).to_string()),
            PickerKind::Theme => self.found_themes().get(picker.selected).map(|info| info.id.to_string()),
        }
    }

    pub fn keep_selected_visible(&mut self, capacity: usize) {
        if capacity == 0 {
            return;
        }
        let rows = form_rows();
        let Some(pos) = rows.iter().position(|r| matches!(r, FormRow::Field(i) if *i == self.selected)) else {
            return;
        };
        let start = if pos > 0 && matches!(rows[pos - 1], FormRow::Header(_)) { pos - 1 } else { pos };
        if start < self.scroll {
            self.scroll = start;
        } else if pos >= self.scroll + capacity {
            self.scroll = pos + 1 - capacity;
        }
        let max_scroll = rows.len().saturating_sub(capacity);
        self.scroll = self.scroll.min(max_scroll);
    }

    pub fn scroll_by(&mut self, delta: i64, capacity: usize) {
        let rows_len = form_rows().len();
        let max_scroll = rows_len.saturating_sub(capacity) as i64;
        self.scroll = (self.scroll as i64 - delta).clamp(0, max_scroll.max(0)) as usize;
    }

    fn edit(&mut self, change: impl FnOnce(&mut String)) {
        if let Some(text) = &mut self.editing {
            change(text);
        }
    }
}

fn flip(field: usize, config: &Config) -> Outcome {
    let next = if value(field, config) == "on" { "off" } else { "on" };
    Outcome::Change(FIELDS[field].key, next.to_string())
}

fn cycle_choice(field: usize, dir: isize, choices: &[&str], config: &Config) -> Outcome {
    let cur = value(field, config);
    let idx = choices.iter().position(|c| *c == cur).unwrap_or(0);
    let len = choices.len() as isize;
    let next = ((idx as isize + dir).rem_euclid(len)) as usize;
    Outcome::Change(FIELDS[field].key, choices[next].to_string())
}

fn cycle_theme(dir: isize, config: &Config) -> Outcome {
    let cur = config.theme.as_deref().unwrap_or("sharky");
    let idx = BUILTINS.iter().position(|info| info.id == cur).unwrap_or(0);
    let len = BUILTINS.len() as isize;
    let next = ((idx as isize + dir).rem_euclid(len)) as usize;
    Outcome::Change("theme", BUILTINS[next].id.to_string())
}

fn step(field: usize, direction: f32, config: &Config) -> Outcome {
    let Field { key, kind: Kind::Number { min, max, step }, .. } = FIELDS[field] else {
        return Outcome::Nothing;
    };
    let current: f32 = value(field, config).parse().unwrap_or(min);
    // Whole steps from zero keep 0.75 from turning into 0.75000006 in the file.
    let next = (((current / step).round() + direction) * step).clamp(min, max);
    if (next - current).abs() < 1e-4 {
        return Outcome::Nothing;
    }
    let digits = if step < 0.1 {
        2
    } else if step < 1.0 {
        1
    } else {
        0
    };
    let text = format!("{next:.digits$}");
    let text = if digits > 0 {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    };
    Outcome::Change(key, text)
}

pub fn value(field: usize, config: &Config) -> String {
    match FIELDS[field].key {
        "theme" => config.theme.clone().unwrap_or_else(|| "sharky".to_string()),
        "matugen" => if config.matugen { "on" } else { "off" }.to_string(),
        "opacity" => config.opacity.to_string(),
        "padding" => config.padding.to_string(),
        "font" => config.font.clone(),
        "font_size" => config.font_size.to_string(),
        "font_weight" => config.font_weight.to_string(),
        "bold_weight" => config.bold_weight.to_string(),
        "italic_style" => if config.oblique { "oblique" } else { "italic" }.to_string(),
        "shell" => config.shell.clone().unwrap_or_default(),
        "scrollback" => config.scrollback.to_string(),
        "clipboard_read" => if config.clipboard_read { "on" } else { "off" }.to_string(),
        _ => String::new(),
    }
}

pub fn display_value(field: usize, config: &Config) -> String {
    let raw = value(field, config);
    match FIELDS[field].kind {
        Kind::Theme => theme_name(&raw).unwrap_or(&raw).to_string(),
        Kind::Text if raw.is_empty() => "Default".to_string(),
        _ => raw,
    }
}

/// The visible slice of a picker list, keeping the selection near the middle.
pub fn visible(selected: usize, count: usize, capacity: usize) -> std::ops::Range<usize> {
    if capacity == 0 {
        return 0..0;
    }
    let first = selected.saturating_sub(capacity / 2).min(count.saturating_sub(capacity));
    first..(first + capacity).min(count)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SettingsView {
    pub container: Rect,
    pub left: Rect,
    pub right: Option<Rect>,
    pub header_y: f32,
    pub list_y: f32,
    pub hint_y: f32,
    pub row_height: f32,
    pub visible_rows: usize,
    pub arrow_width: f32,
    pub scale: f32,
}

impl SettingsView {
    pub fn new(width: f32, height: f32, strip: f32, scale: f32, cell_height: f32) -> Self {
        let pad = 20.0 * scale;
        let max_w = (1120.0 * scale).min((width - pad * 2.0).max(200.0));
        let container_x = ((width - max_w) / 2.0).max(pad * 0.5);
        let container_y = strip + pad * 0.75;
        let container_h = (height - container_y - pad * 0.75).max(120.0);
        let container = Rect { x: container_x, y: container_y, width: max_w, height: container_h };

        let row_height = (cell_height + 9.0 * scale).round().max(18.0);
        let header_y = container.y;
        let header_h = row_height + 6.0 * scale;
        let hint_h = row_height;
        let list_y = container.y + header_h;
        let hint_y = container.y + container.height - hint_h;
        let list_h = (hint_y - list_y - 6.0 * scale).max(row_height);

        let two_col = max_w >= 720.0 * scale;
        let col_gap = 24.0 * scale;
        let (left, right) = if two_col {
            let left_w = ((max_w - col_gap) * 0.54).round();
            let right_x = container.x + left_w + col_gap;
            let right_w = (container.x + max_w - right_x).max(120.0);
            (
                Rect { x: container.x, y: list_y, width: left_w, height: list_h },
                Some(Rect { x: right_x, y: list_y, width: right_w, height: list_h }),
            )
        } else {
            (Rect { x: container.x, y: list_y, width: max_w, height: list_h }, None)
        };
        let visible_rows = ((list_h / row_height) as usize).max(1);
        let arrow_width = (22.0 * scale).round();

        Self {
            container,
            left,
            right,
            header_y,
            list_y,
            hint_y,
            row_height,
            visible_rows,
            arrow_width,
            scale,
        }
    }

    pub fn inset(&self) -> f32 {
        12.0 * self.scale
    }

    /// Returns `(less_arrow_x, more_arrow_x)` within `self.left` for stepped/cycled fields.
    pub fn arrows(&self, kind: Kind) -> Option<(f32, f32)> {
        let more_x = self.left.x + self.left.width - self.inset() - self.arrow_width;
        let value_span = match kind {
            Kind::Number { .. } => 68.0 * self.scale,
            Kind::Choice(_) => 82.0 * self.scale,
            Kind::Theme => 148.0 * self.scale,
            _ => return None,
        };
        let less_x = (more_x - value_span - self.arrow_width).max(self.left.x + self.left.width * 0.42);
        Some((less_x, more_x))
    }

    pub fn picker_rows(&self) -> usize {
        self.visible_rows.saturating_sub(1).max(1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    Row(usize),
    Less(usize),
    More(usize),
    Inside,
    Outside,
}

pub fn tab_zone(view: SettingsView, x: f32, y: f32, settings: &Settings) -> Zone {
    let left = view.left;
    if !(left.x..left.x + left.width).contains(&x) || !(left.y..left.y + left.height).contains(&y) {
        return Zone::Outside;
    }
    let visual = ((y - left.y) / view.row_height) as usize;
    if settings.picker.is_some() {
        let Some(row) = visual.checked_sub(1) else {
            return Zone::Inside;
        };
        return if row < view.picker_rows() { Zone::Row(row) } else { Zone::Inside };
    }

    let rows = form_rows();
    let max_scroll = rows.len().saturating_sub(view.visible_rows);
    let scroll = settings.scroll.min(max_scroll);
    let Some(FormRow::Field(field_idx)) = rows.get(scroll + visual).copied() else {
        return Zone::Inside;
    };
    if let Some((less_x, more_x)) = view.arrows(FIELDS[field_idx].kind) {
        if x >= more_x && x <= more_x + view.arrow_width {
            return Zone::More(field_idx);
        }
        if x >= less_x && x <= less_x + view.arrow_width {
            return Zone::Less(field_idx);
        }
    }
    Zone::Row(field_idx)
}

use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Text,
    Font,
    Number { min: f32, max: f32, step: f32 },
    Toggle,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Field {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: Kind,
}

pub const FIELDS: [Field; 6] = [
    Field { key: "font", label: "Font", kind: Kind::Font },
    Field { key: "font_size", label: "Font size", kind: Kind::Number { min: 6.0, max: 72.0, step: 0.5 } },
    Field { key: "opacity", label: "Opacity", kind: Kind::Number { min: 0.1, max: 1.0, step: 0.05 } },
    Field { key: "padding", label: "Padding", kind: Kind::Number { min: 0.0, max: 64.0, step: 2.0 } },
    Field { key: "shell", label: "Shell for new tabs", kind: Kind::Text },
    Field { key: "matugen", label: "Colours from matugen", kind: Kind::Toggle },
];

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

#[derive(Debug, Default)]
pub struct Settings {
    pub selected: usize,
    pub editing: Option<String>,
    pub picker: Option<Picker>,
    pub fonts: Vec<String>,
}

/// The installed fonts in place of the fields. `selected` counts inside the filtered list.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Picker {
    pub query: String,
    pub selected: usize,
}

impl Settings {
    pub fn handle(&mut self, command: Command, config: &Config) -> Outcome {
        if self.editing.is_some() {
            return self.handle_editing(command);
        }
        if self.picker.is_some() {
            return self.handle_picker(command);
        }
        let toggle = FIELDS[self.selected].kind == Kind::Toggle;
        match command {
            Command::Less | Command::More | Command::Confirm if toggle => return flip(self.selected, config),
            Command::Up => self.selected = (self.selected + FIELDS.len() - 1) % FIELDS.len(),
            Command::Down => self.selected = (self.selected + 1) % FIELDS.len(),
            Command::Less => return step(self.selected, -1.0, config),
            Command::More => return step(self.selected, 1.0, config),
            Command::Confirm if FIELDS[self.selected].kind == Kind::Text => {
                self.editing = Some(value(self.selected, config));
            }
            Command::Confirm if FIELDS[self.selected].kind == Kind::Font => {
                let selected = self.fonts.iter().position(|name| *name == config.font).unwrap_or(0);
                self.picker = Some(Picker { query: String::new(), selected });
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
        let chosen = self.chosen().map(str::to_string);
        let count = self.found().len().max(1);
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
                if let Some(name) = chosen {
                    self.picker = None;
                    return Outcome::Change("font", name);
                }
            }
            _ => {}
        }
        Outcome::Nothing
    }

    /// The fonts that match what was typed.
    pub fn found(&self) -> Vec<&str> {
        let query = self.picker.as_ref().map_or(String::new(), |picker| picker.query.to_lowercase());
        self.fonts.iter().map(String::as_str).filter(|name| name.to_lowercase().contains(&query)).collect()
    }

    /// Selects the font drawn on a row of the panel. False when the row is empty.
    pub fn pick(&mut self, row: usize) -> bool {
        let count = self.found().len();
        let Some(picker) = &mut self.picker else {
            return false;
        };
        let index = visible(picker.selected, count).nth(row);
        picker.selected = index.unwrap_or(picker.selected);
        index.is_some()
    }

    fn chosen(&self) -> Option<&str> {
        self.found().get(self.picker.as_ref()?.selected).copied()
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

fn step(field: usize, direction: f32, config: &Config) -> Outcome {
    let Field { key, kind: Kind::Number { min, max, step }, .. } = FIELDS[field] else {
        return Outcome::Nothing;
    };
    let current: f32 = value(field, config).parse().unwrap_or(min);
    // Whole steps from zero keep 0.75 from turning into 0.75000006 in the file.
    let next = (((current / step).round() + direction) * step).clamp(min, max);
    if next == current {
        return Outcome::Nothing;
    }
    let digits = if step < 0.1 { 2 } else { 1 };
    let text = format!("{next:.digits$}");
    let text = text.trim_end_matches('0').trim_end_matches('.').to_string();
    Outcome::Change(key, text)
}

pub fn value(field: usize, config: &Config) -> String {
    match FIELDS[field].key {
        "font" => config.font.clone(),
        "font_size" => config.font_size.to_string(),
        "opacity" => config.opacity.to_string(),
        "padding" => config.padding.to_string(),
        "matugen" => if config.matugen { "on" } else { "off" }.to_string(),
        _ => config.shell.clone().unwrap_or_default(),
    }
}

/// The part of a list the panel shows, with the selection near the middle.
pub fn visible(selected: usize, count: usize) -> std::ops::Range<usize> {
    let first = selected.saturating_sub(FIELDS.len() / 2).min(count.saturating_sub(FIELDS.len()));
    first..(first + FIELDS.len()).min(count)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Panel {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub row_height: f32,
}

impl Panel {
    /// One extra row holds the hint line.
    pub fn height(&self) -> f32 {
        (FIELDS.len() + 1) as f32 * self.row_height
    }

    pub fn arrow_width(&self) -> f32 {
        self.row_height
    }

    /// A number reads `‹ value ›` at the right end, with room for its digits.
    pub fn less_x(&self) -> f32 {
        self.x + self.width - 4.0 * self.arrow_width()
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

pub fn zone(panel: Panel, x: f32, y: f32) -> Zone {
    let inside = (panel.x..panel.x + panel.width).contains(&x) && (panel.y..panel.y + panel.height()).contains(&y);
    if !inside {
        return Zone::Outside;
    }
    let row = ((y - panel.y) / panel.row_height) as usize;
    if row >= FIELDS.len() {
        return Zone::Inside;
    }
    if matches!(FIELDS[row].kind, Kind::Number { .. }) {
        if x >= panel.x + panel.width - panel.arrow_width() {
            return Zone::More(row);
        }
        if (panel.less_x()..panel.less_x() + panel.arrow_width()).contains(&x) {
            return Zone::Less(row);
        }
    }
    Zone::Row(row)
}

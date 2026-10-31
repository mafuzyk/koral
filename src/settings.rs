use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Text,
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
    Field { key: "font", label: "Font", kind: Kind::Text },
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
}

impl Settings {
    pub fn handle(&mut self, command: Command, config: &Config) -> Outcome {
        if self.editing.is_some() {
            return self.handle_editing(command);
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

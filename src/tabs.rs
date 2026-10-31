use crate::blocks::{Activity, BlockStore};

const MAX_TAB: f32 = 220.0;
const LEFT_INSET: f32 = 8.0;
pub const CLOSE_ZONE: f32 = 28.0;

pub struct Tabs<T> {
    items: Vec<T>,
    active: usize,
}

impl<T> Tabs<T> {
    pub fn new(first: T) -> Self {
        Self { items: vec![first], active: 0 }
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    pub fn active_index(&self) -> usize {
        self.active
    }
    pub fn active(&self) -> Option<&T> {
        self.items.get(self.active)
    }
    pub fn active_mut(&mut self) -> Option<&mut T> {
        self.items.get_mut(self.active)
    }
    pub fn push(&mut self, item: T) {
        self.items.push(item);
        self.active = self.items.len() - 1;
    }
    pub fn close(&mut self, index: usize) -> Option<T> {
        if index >= self.items.len() {
            return None;
        }
        let item = self.items.remove(index);
        if index < self.active {
            self.active -= 1;
        }
        self.active = self.active.min(self.items.len().saturating_sub(1));
        Some(item)
    }
    pub fn select(&mut self, index: usize) {
        if index < self.items.len() {
            self.active = index;
        }
    }
    pub fn next(&mut self) {
        self.active = (self.active + 1) % self.items.len();
    }
    pub fn previous(&mut self) {
        self.active = (self.active + self.items.len() - 1) % self.items.len();
    }
    pub fn move_tab(&mut self, from: usize, to: usize) {
        if from >= self.items.len() || to >= self.items.len() {
            return;
        }
        let item = self.items.remove(from);
        self.items.insert(to, item);
        self.active = match self.active {
            active if active == from => to,
            active if from < active && to >= active => active - 1,
            active if from > active && to <= active => active + 1,
            active => active,
        };
    }
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.items.iter()
    }
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.items.iter_mut()
    }
}

pub fn title(blocks: &BlockStore, home: Option<&str>) -> String {
    let Some(latest) = blocks.latest() else {
        return "koral".to_string();
    };
    if blocks.activity() == Activity::Running
        && let Some(line) = latest.command.as_deref().and_then(|command| command.lines().next())
    {
        return line.to_string();
    }
    match latest.cwd.as_deref() {
        None => "koral".to_string(),
        Some(cwd) if Some(cwd) == home => "~".to_string(),
        Some(cwd) => cwd.rsplit('/').next().filter(|name| !name.is_empty()).unwrap_or("/").to_string(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Bar {
    pub tabs: Vec<Rect>,
    pub new_tab: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Tab(usize),
    Close(usize),
    New,
    Empty,
}

impl Rect {
    fn contains(&self, x: f32, y: f32) -> bool {
        (self.x..self.x + self.width).contains(&x) && (self.y..self.y + self.height).contains(&y)
    }
}

// shortcut: tabs keep shrinking past legibility, add scrolling if that ever hurts.
pub fn layout(count: usize, width: f32, height: f32) -> Bar {
    let room = (width - LEFT_INSET - height).max(0.0);
    let tab = (room / count.max(1) as f32).min(MAX_TAB);
    let tabs: Vec<Rect> = (0..count)
        .map(|index| Rect { x: LEFT_INSET + index as f32 * tab, y: 0.0, width: tab, height })
        .collect();
    let new_tab = Rect { x: LEFT_INSET + count as f32 * tab, y: 0.0, width: height, height };
    Bar { tabs, new_tab }
}

pub fn hit(bar: &Bar, x: f32, y: f32) -> Hit {
    if bar.new_tab.contains(x, y) {
        return Hit::New;
    }
    match bar.tabs.iter().position(|tab| tab.contains(x, y)) {
        Some(index) if x >= bar.tabs[index].x + bar.tabs[index].width - CLOSE_ZONE => Hit::Close(index),
        Some(index) => Hit::Tab(index),
        None => Hit::Empty,
    }
}

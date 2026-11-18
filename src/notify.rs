use std::process::Command;
use std::time::{Duration, Instant};

const SPACING: Duration = Duration::from_secs(2);
pub const TITLE_LIMIT: usize = 256;
pub const BODY_LIMIT: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub title: String,
    pub body: String,
}

/// Any program can send these, so they stay rare.
#[derive(Default)]
pub struct Limiter {
    last: Option<Instant>,
    held: Option<Notice>,
}

impl Limiter {
    pub fn allow(&mut self, now: Instant) -> bool {
        let allowed = self.last.is_none_or(|last| now.duration_since(last) >= SPACING);
        if allowed {
            self.last = Some(now);
        }
        allowed
    }

    /// A suppressed notice waits for the window instead of being lost; the newest wins.
    pub fn hold(&mut self, notice: Notice) {
        self.held = Some(notice);
    }

    /// The held notice once the spacing has passed.
    pub fn release(&mut self, now: Instant) -> Option<Notice> {
        if now.duration_since(self.last?) < SPACING {
            return None;
        }
        self.last = Some(now);
        self.held.take()
    }

    /// When a held notice may be shown.
    pub fn due(&self) -> Option<Instant> {
        self.held.as_ref()?;
        self.last.map(|last| last + SPACING)
    }
}

pub fn clean(text: &str, limit: usize) -> String {
    text.chars().filter(|ch| !ch.is_control()).take(limit).collect()
}

pub fn show(notice: &Notice) {
    let title = if notice.title.is_empty() { "koral" } else { &notice.title };
    let spawned = Command::new("notify-send").args(["--app-name=koral", "--icon=koral", "--", title, &notice.body]).spawn();
    if let Ok(mut child) = spawned {
        std::thread::spawn(move || child.wait());
    }
}

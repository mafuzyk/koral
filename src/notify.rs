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
}

impl Limiter {
    pub fn allow(&mut self, now: Instant) -> bool {
        let allowed = self.last.is_none_or(|last| now.duration_since(last) >= SPACING);
        if allowed {
            self.last = Some(now);
        }
        allowed
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

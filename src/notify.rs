use std::process::Command;
use std::time::{Duration, Instant};

use crate::config::PermissionPolicy;

const SPACING: Duration = Duration::from_secs(2);
pub const TITLE_LIMIT: usize = 256;
pub const BODY_LIMIT: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub title: String,
    pub body: String,
}

/// What the caller must do with a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// The operation may be performed right away.
    Performed,
    /// The operation was held for the user or dropped by policy.
    Confirm,
    /// The operation was refused.
    Denied,
    /// A rate limit deferred the operation.
    Held,
}

/// Who requested a clipboard write. Only programs are subject to permission policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOrigin {
    /// An explicit copy command from the user, such as a selection or block output.
    User,
    /// A program request, such as OSC 52 or OSC 5522.
    Program,
}

/// Pure gating rule for clipboard writes, independent of notifier state.
pub fn clipboard_write_decision(origin: WriteOrigin, policy: PermissionPolicy) -> Decision {
    match origin {
        WriteOrigin::User => Decision::Performed,
        WriteOrigin::Program => match policy {
            PermissionPolicy::Allow => Decision::Performed,
            PermissionPolicy::Ask => Decision::Confirm,
            PermissionPolicy::Deny => Decision::Denied,
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pending {
    Clipboard(String),
    Notice(Notice),
}

impl Pending {
    fn prompt(&self) -> &'static str {
        match self {
            Self::Clipboard(_) => "Allow clipboard write?",
            Self::Notice(_) => "Allow notification?",
        }
    }
}

/// Legacy spacing helper kept for direct tests; the notifier tracks delivery itself.
#[derive(Default)]
pub struct Limiter {
    last: Option<Instant>,
    held: Option<Notice>,
}

impl Limiter {
    pub fn allow(&mut self, now: Instant) -> bool {
        let allowed = self
            .last
            .is_none_or(|last| now.duration_since(last) >= SPACING);
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

/// One policy-gated request at a time: a confirmation owns the screen and the clipboard.
pub struct Notifier {
    clipboard_write: PermissionPolicy,
    notifications: PermissionPolicy,
    pending: Option<Pending>,
    ready: Vec<Pending>,
    asked_at: Option<Instant>,
    queued_notice: Option<(Notice, Instant)>,
    last_delivered_at: Option<Instant>,
}

impl Notifier {
    pub fn new(policy: PermissionPolicy) -> Self {
        Self::with_policies(policy, policy)
    }

    pub fn with_policies(
        clipboard_write: PermissionPolicy,
        notifications: PermissionPolicy,
    ) -> Self {
        Self {
            clipboard_write,
            notifications,
            ..Self::default()
        }
    }

    pub fn set_policy(&mut self, policy: PermissionPolicy) {
        self.set_policies(policy, policy);
    }

    /// A request authorized under one policy must not survive a policy change.
    pub fn set_policies(
        &mut self,
        clipboard_write: PermissionPolicy,
        notifications: PermissionPolicy,
    ) {
        if (self.clipboard_write, self.notifications) == (clipboard_write, notifications) {
            return;
        }
        if self.clipboard_write != clipboard_write {
            self.clipboard_write = clipboard_write;
            if matches!(self.pending, Some(Pending::Clipboard(_))) {
                self.pending = None;
                self.asked_at = None;
            }
            self.ready
                .retain(|pending| !matches!(pending, Pending::Clipboard(_)));
        }
        if self.notifications != notifications {
            self.notifications = notifications;
            if matches!(self.pending, Some(Pending::Notice(_))) {
                self.pending = None;
                self.asked_at = None;
            }
            self.ready
                .retain(|pending| !matches!(pending, Pending::Notice(_)));
            self.queued_notice = None;
        }
    }

    pub fn policy(&self) -> PermissionPolicy {
        self.clipboard_write
    }

    pub fn policies(&self) -> (PermissionPolicy, PermissionPolicy) {
        (self.clipboard_write, self.notifications)
    }

    /// The delivery window for a notice accepted at `now`.
    fn notice_due(&self, now: Instant) -> Instant {
        self.last_delivered_at
            .map_or(now, |last| (last + SPACING).max(now))
    }

    pub fn clipboard_write(&mut self, text: String, now: Instant) -> Decision {
        self.clipboard_write_from(WriteOrigin::Program, text, now)
    }

    /// User writes bypass policy and any pending prompt; program writes are gated.
    pub fn clipboard_write_from(
        &mut self,
        origin: WriteOrigin,
        text: String,
        now: Instant,
    ) -> Decision {
        if origin == WriteOrigin::Program && self.pending.is_some() {
            return Decision::Denied;
        }
        let decision = clipboard_write_decision(origin, self.clipboard_write);
        match decision {
            Decision::Performed => self.ready.push(Pending::Clipboard(text)),
            Decision::Confirm => {
                self.pending = Some(Pending::Clipboard(text));
                self.asked_at = Some(now);
            }
            _ => {}
        }
        decision
    }

    pub fn notify(&mut self, notice: Notice, now: Instant) -> Decision {
        let notice = Notice {
            title: clean(&notice.title, TITLE_LIMIT),
            body: clean(&notice.body, BODY_LIMIT),
        };
        self.request(Pending::Notice(notice), now)
    }

    fn request(&mut self, request: Pending, now: Instant) -> Decision {
        if self.pending.is_some() {
            return Decision::Denied;
        }
        match self.notifications {
            PermissionPolicy::Deny => Decision::Denied,
            PermissionPolicy::Ask => {
                self.pending = Some(request);
                self.asked_at = Some(now);
                Decision::Confirm
            }
            PermissionPolicy::Allow => {
                if let Pending::Notice(notice) = request {
                    let due = self.notice_due(now);
                    self.queued_notice = Some((notice, due));
                    if due > now {
                        return Decision::Held;
                    }
                } else {
                    self.ready.push(request);
                }
                Decision::Performed
            }
        }
    }

    /// The prompt the UI should show while a request waits.
    pub fn take_confirmation(&mut self) -> Option<String> {
        self.pending
            .as_ref()
            .map(|pending| pending.prompt().to_string())
    }

    pub fn confirmation_due(&self) -> Option<Instant> {
        self.asked_at.map(|at| at + Duration::from_secs(3))
    }

    pub fn confirm(&mut self, now: Instant) -> Option<Decision> {
        let at = self.asked_at.take()?;
        if now.duration_since(at) >= Duration::from_secs(3) {
            self.pending = None;
            return Some(Decision::Denied);
        }
        self.accept_pending(now)
    }

    pub fn cancel(&mut self) -> Option<Decision> {
        self.pending.take()?;
        self.asked_at = None;
        Some(Decision::Denied)
    }

    fn accept_pending(&mut self, now: Instant) -> Option<Decision> {
        let request = self.pending.take()?;
        self.asked_at = None;
        if let Pending::Notice(notice) = request {
            let due = self.notice_due(now);
            self.queued_notice = Some((notice, due));
        } else {
            self.ready.push(request);
        }
        Some(Decision::Performed)
    }

    /// The next clipboard text the UI may write, if any.
    pub fn take_ready_text(&mut self) -> Option<String> {
        let index = self
            .ready
            .iter()
            .position(|pending| matches!(pending, Pending::Clipboard(_)))?;
        match self.ready.remove(index) {
            Pending::Clipboard(text) => Some(text),
            _ => None,
        }
    }

    /// The next notice that is due to be shown, if any.
    pub fn take_ready_notice(&mut self, now: Instant) -> Option<Notice> {
        let (_, due) = self.queued_notice.as_ref()?;
        if now < *due {
            return None;
        }
        let (notice, _) = self.queued_notice.take()?;
        self.last_delivered_at = Some(now);
        Some(notice)
    }

    /// When the waiting notification may be shown, if one is queued.
    pub fn notification_due(&self) -> Option<Instant> {
        self.queued_notice.as_ref().map(|(_, due)| *due)
    }
}

impl Default for Notifier {
    fn default() -> Self {
        Self {
            clipboard_write: PermissionPolicy::Ask,
            notifications: PermissionPolicy::Ask,
            pending: None,
            ready: Vec::new(),
            asked_at: None,
            queued_notice: None,
            last_delivered_at: None,
        }
    }
}

pub fn clean(text: &str, limit: usize) -> String {
    text.chars()
        .filter(|ch| !ch.is_control())
        .take(limit)
        .collect()
}

pub fn show(notice: &Notice) {
    let title = if notice.title.is_empty() {
        "koral"
    } else {
        &notice.title
    };
    let spawned = Command::new("notify-send")
        .args([
            "--app-name=koral",
            "--icon=koral",
            "--",
            title,
            &notice.body,
        ])
        .spawn();
    if let Ok(mut child) = spawned {
        std::thread::spawn(move || child.wait());
    }
}

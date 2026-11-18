use std::collections::HashMap;
use std::fs::File;
use std::io::{ErrorKind, Read, Write};
use std::path::PathBuf;
use std::process::ExitStatus;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, EventListener, OnResize, WindowSize};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::tty::{self, ChildEvent, EventedPty, EventedReadWrite, Options, Pty, Shell};
use alacritty_terminal::vte::ansi::Rgb;
use polling::{Event as PollEvent, Events, PollMode, Poller};
use glyphon::FontSystem;
use winit::event_loop::EventLoopProxy;

use crate::notify::Notice;
use crate::tabs::clean_title;
use crate::session::Session;

const SCROLLBACK: usize = 10_000;
const FRAME: Duration = Duration::from_millis(16);
/// Longest a writer waits on a full PTY before checking whether the tab still exists.
const WRITE_WAIT: Duration = Duration::from_millis(100);

type ColorReply = Arc<dyn Fn(Rgb) -> String + Send + Sync>;

pub fn term_config() -> Config {
    Config { scrolling_history: SCROLLBACK, kitty_keyboard: true, ..Config::default() }
}

pub enum UserEvent {
    Redraw,
    Exited(u64),
    Reply(u64, String),
    Copy(String),
    Color(u64, usize, ColorReply),
    Colors,
    Blink,
    Title(u64, Option<String>),
    Bell(u64),
    Notify(u64, Notice),
    Size(u64, SizeReply),
    Fonts(Box<FontSystem>, Vec<String>),
}

pub type SizeReply = Arc<dyn Fn(WindowSize) -> String + Sync + Send>;

/// alacritty answers as a VT102, and kitty's tools wait out a timeout on so short an answer.
pub fn device_reply(text: String) -> String {
    if text == "\x1b[?6c" { "\x1b[?62;c".to_string() } else { text }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub columns: usize,
    pub lines: usize,
    pub cell_width: u16,
    pub cell_height: u16,
}

impl Geometry {
    fn term_size(self) -> TermSize {
        TermSize::new(self.columns, self.lines)
    }

    pub fn window_size(self) -> WindowSize {
        WindowSize {
            num_lines: self.lines as u16,
            num_cols: self.columns as u16,
            cell_width: self.cell_width,
            cell_height: self.cell_height,
        }
    }
}

#[derive(Clone)]
pub struct Listener {
    tab: u64,
    proxy: EventLoopProxy<UserEvent>,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let event = match event {
            Event::PtyWrite(text) => UserEvent::Reply(self.tab, device_reply(text)),
            Event::TextAreaSizeRequest(reply) => UserEvent::Size(self.tab, reply),
            Event::ClipboardStore(_, text) => UserEvent::Copy(text),
            Event::ColorRequest(index, reply) => UserEvent::Color(self.tab, index, reply),
            Event::CursorBlinkingChange => UserEvent::Blink,
            Event::Title(text) => UserEvent::Title(self.tab, clean_title(&text)),
            Event::ResetTitle => UserEvent::Title(self.tab, None),
            Event::Bell => UserEvent::Bell(self.tab),
            _ => return,
        };
        let _ = self.proxy.send_event(event);
    }
}

pub struct Terminal {
    pub id: u64,
    pub session: Arc<FairMutex<Session<Listener>>>,
    pub scroll: usize,
    pub title: Option<String>,
    pub rang: bool,
    pty: Arc<Mutex<Pty>>,
    writer: Sender<Vec<u8>>,
    wake: Arc<Poller>,
    closed: Arc<AtomicBool>,
}

impl Terminal {
    pub fn spawn(
        id: u64,
        proxy: EventLoopProxy<UserEvent>,
        shell: &str,
        geometry: Geometry,
        cwd: Option<PathBuf>,
    ) -> std::io::Result<Self> {
        let listener = Listener { tab: id, proxy: proxy.clone() };
        let term = Term::new(term_config(), &geometry.term_size(), listener);
        let mut session = Session::new(term);
        session.graphics.set_cell(geometry.cell_width.into(), geometry.cell_height.into());
        let session = Arc::new(FairMutex::new(session));

        // shortcut: no koral terminfo entry, add one if an app needs it.
        let env = HashMap::from([
            ("TERM".to_string(), "xterm-256color".to_string()),
            ("COLORTERM".to_string(), "truecolor".to_string()),
            ("TERM_PROGRAM".to_string(), "koral".to_string()),
        ]);
        let options = Options {
            shell: Some(Shell::new(shell.to_string(), Vec::new())),
            working_directory: cwd,
            drain_on_exit: false,
            env,
        };
        let pty = Arc::new(Mutex::new(tty::new(&options, geometry.window_size(), id)?));

        let (writer, inbox) = channel::<Vec<u8>>();
        let output = locked(&pty).file().try_clone()?;
        std::thread::spawn(move || write_loop(output, inbox));

        let wake = Arc::new(Poller::new()?);
        let closed = Arc::new(AtomicBool::new(false));
        let reader_pty = pty.clone();
        let reader_session = session.clone();
        let (reader_wake, reader_closed) = (wake.clone(), closed.clone());
        std::thread::spawn(move || read_loop(id, reader_pty, reader_session, proxy, reader_wake, reader_closed));

        Ok(Self { id, session, scroll: 0, title: None, rang: false, pty, writer, wake, closed })
    }

    pub fn write(&self, bytes: Vec<u8>) {
        let _ = self.writer.send(bytes);
    }

    pub fn resize(&mut self, geometry: Geometry) {
        let mut session = self.session.lock();
        session.term.resize(geometry.term_size());
        session.graphics.set_cell(geometry.cell_width.into(), geometry.cell_height.into());
        drop(session);
        locked(&self.pty).on_resize(geometry.window_size());
    }
}

/// A poisoned lock must not take the window thread down with it.
fn locked(pty: &Mutex<Pty>) -> MutexGuard<'_, Pty> {
    pty.lock().unwrap_or_else(|error| error.into_inner())
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // Wake the reader so it releases the PTY, which then hangs up the child.
        self.closed.store(true, Ordering::SeqCst);
        let _ = self.wake.notify();
    }
}

/// Drains the input channel into the PTY, and stops as soon as the tab is gone.
pub fn write_loop(mut output: File, inbox: Receiver<Vec<u8>>) {
    let poller = Poller::new().ok();
    if let Some(poller) = &poller {
        // SAFETY: `output` outlives the poller, both are dropped together in this thread.
        unsafe { let _ = poller.add(&output, PollEvent::writable(0)); }
    }
    let mut events = Events::new();
    let mut pending: Vec<u8> = Vec::new();
    let mut sent = 0;
    loop {
        if sent == pending.len() {
            pending.clear();
            sent = 0;
            match inbox.recv() {
                Ok(bytes) => pending = bytes,
                Err(_) => return,
            }
            continue;
        }
        // The PTY is non-blocking, and a slow child must never stall the window thread.
        match output.write(&pending[sent..]) {
            Ok(0) => return,
            Ok(written) => sent += written,
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                // Bounded: a paused reader must not pin the thread once the tab is gone.
                match &poller {
                    Some(poller) => {
                        events.clear();
                        let _ = poller.modify(&output, PollEvent::writable(0));
                        let _ = poller.wait(&mut events, Some(WRITE_WAIT));
                    }
                    None => std::thread::sleep(WRITE_WAIT),
                }
                match inbox.try_recv() {
                    Err(TryRecvError::Disconnected) => return,
                    Ok(bytes) => pending.extend(bytes),
                    Err(TryRecvError::Empty) => {}
                }
            }
            Err(_) => return,
        }
    }
}

/// When the window should repaint while a PTY is being read.
#[derive(Debug, Default)]
pub struct Pacer {
    painted: Option<Instant>,
    held: bool,
}

impl Pacer {
    pub fn read(&mut self, now: Instant) -> bool {
        let due = self.painted.is_none_or(|at| now.duration_since(at) >= FRAME);
        self.held = !due;
        if due {
            self.painted = Some(now);
        }
        due
    }

    pub fn drained(&mut self) -> bool {
        self.painted = None;
        std::mem::take(&mut self.held)
    }
}

/// Drives the PTY, handing each chunk of output and the end of each burst to `sink`.
///
/// Returns the child's exit status when the child is seen to exit on its own.
pub fn pump(
    pty: &Mutex<Pty>,
    poller: &Arc<Poller>,
    closed: &AtomicBool,
    mut sink: impl FnMut(Option<&[u8]>),
) -> Option<ExitStatus> {
    // Registers the master and the SIGCHLD pipe, so a wake means either side has news.
    // SAFETY: the PTY outlives the poller, which lives as long as the tab.
    unsafe { locked(pty).register(poller, PollEvent::readable(0), PollMode::Level).ok()? };

    let mut events = Events::new();
    let mut buffer = vec![0; 64 * 1024];
    loop {
        events.clear();
        if poller.wait(&mut events, None).is_err() || closed.load(Ordering::SeqCst) {
            return None;
        }
        // The child's death is authoritative: a job left behind can hold the slave open for good.
        let exit = match locked(pty).next_child_event() {
            Some(ChildEvent::Exited(status)) => Some(status),
            None => None,
        };
        loop {
            let read = {
                let mut pty = locked(pty);
                pty.reader().read(&mut buffer)
            };
            match read {
                Ok(0) => return exit.flatten(),
                Ok(count) => sink(Some(&buffer[..count])),
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == ErrorKind::Interrupted => {}
                Err(_) => return exit.flatten(),
            }
        }
        sink(None);
        if exit.is_some() {
            return exit.flatten();
        }
        if locked(pty).reregister(poller, PollEvent::readable(0), PollMode::Level).is_err() {
            return None;
        }
    }
}

fn read_loop(
    id: u64,
    pty: Arc<Mutex<Pty>>,
    session: Arc<FairMutex<Session<Listener>>>,
    proxy: EventLoopProxy<UserEvent>,
    poller: Arc<Poller>,
    closed: Arc<AtomicBool>,
) {
    let mut pacer = Pacer::default();
    let _ = pump(&pty, &poller, &closed, |chunk| match chunk {
        Some(bytes) => {
            let mut session = session.lock();
            session.feed(bytes, Instant::now());
            for reply in session.replies.drain(..) {
                let _ = proxy.send_event(UserEvent::Reply(id, reply));
            }
            for notice in session.notices.drain(..) {
                let _ = proxy.send_event(UserEvent::Notify(id, notice));
            }
            drop(session);
            if pacer.read(Instant::now()) {
                let _ = proxy.send_event(UserEvent::Redraw);
            }
        }
        None => {
            if pacer.drained() {
                let _ = proxy.send_event(UserEvent::Redraw);
            }
        }
    });
    let _ = proxy.send_event(UserEvent::Exited(id));
}

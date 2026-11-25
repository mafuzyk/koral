use std::collections::HashMap;
use std::fs::File;
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, EventListener, OnResize, WindowSize};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{ClipboardType, Config, Term};
use alacritty_terminal::tty::{
    self, ChildEvent, EventedPty, EventedReadWrite, Options, Pty, Shell,
};
use alacritty_terminal::vte::ansi::Rgb;
use glyphon::FontSystem;
use polling::{Event as PollEvent, Events, PollMode, Poller};
use winit::event_loop::EventLoopProxy;

use crate::notify::Notice;
use crate::session::Session;
use crate::tabs::clean_title;
use crate::view::{Row, RowProjection};

const SCROLLBACK: usize = 10_000;
const DEFAULT_REFRESH_MILLIHERTZ: u32 = 60_000;
/// Longest a writer waits on a full PTY before checking whether the tab still exists.
const WRITE_WAIT: Duration = Duration::from_millis(100);
/// An adaptive staging buffer stops growing at 1 MiB so a burst cannot pin the RSS.
const MAX_STAGE: usize = 1024 * 1024;

type ColorReply = Arc<dyn Fn(Rgb) -> String + Send + Sync>;
pub type PasteReply = Arc<dyn Fn(&str) -> String + Send + Sync>;

pub fn term_config() -> Config {
    term_config_with(SCROLLBACK)
}

pub fn term_config_with(scrollback: usize) -> Config {
    Config {
        scrolling_history: scrollback,
        kitty_keyboard: true,
        ..Config::default()
    }
}

pub enum UserEvent {
    Redraw,
    Exited(u64),
    Reply(u64, String),
    Copy(String),
    PasteRequest(u64, ClipboardType, PasteReply),
    Color(u64, usize, ColorReply),
    Colors,
    Config,
    Blink,
    Title(u64, Option<String>),
    Bell(u64),
    Notify(u64, Notice),
    ClipboardOp(u64, crate::marks::ClipboardOp),
    BlockEvent(u64, crate::ipc::BlockEvent),
    Size(u64, SizeReply),
    Fonts(Box<FontSystem>, Vec<String>),
    Ipc(crate::ipc::Request, Sender<String>),
    AccessKit(accesskit_winit::Event),
}

impl From<accesskit_winit::Event> for UserEvent {
    fn from(event: accesskit_winit::Event) -> Self {
        Self::AccessKit(event)
    }
}

pub type SizeReply = Arc<dyn Fn(WindowSize) -> String + Sync + Send>;

/// alacritty answers as a VT102, and kitty's tools wait out a timeout on so short an answer.
pub fn device_reply(text: String) -> String {
    if text == "\x1b[?6c" {
        "\x1b[?62;c".to_string()
    } else {
        text
    }
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
            Event::ClipboardLoad(kind, reply) => UserEvent::PasteRequest(self.tab, kind, reply),
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
    projection: Mutex<RowProjection>,
    pub scroll: usize,
    pub title: Option<String>,
    pub rang: bool,
    pty: Option<Arc<Mutex<Pty>>>,
    writer: Sender<Vec<u8>>,
    wake: Arc<Poller>,
    closed: Arc<AtomicBool>,
    refresh_millihertz: Arc<AtomicU32>,
}

impl Terminal {
    /// The caller holds this terminal's session lock for a consistent view of the rows.
    pub fn projected_rows(&self, session: &Session<Listener>) -> Arc<Vec<Row>> {
        let mut projection = self
            .projection
            .lock()
            .expect("row projection lock poisoned");
        projection.refresh(&session.term, &session.blocks, session.grid_revision());
        projection.shared()
    }

    pub fn spawn(
        id: u64,
        proxy: EventLoopProxy<UserEvent>,
        shell: &str,
        geometry: Geometry,
        cwd: Option<PathBuf>,
    ) -> std::io::Result<Self> {
        Self::spawn_with_args(id, proxy, shell, &[], geometry, cwd)
    }

    pub fn spawn_with_args(
        id: u64,
        proxy: EventLoopProxy<UserEvent>,
        shell: &str,
        args: &[String],
        geometry: Geometry,
        cwd: Option<PathBuf>,
    ) -> std::io::Result<Self> {
        Self::spawn_configured(id, proxy, shell, args, geometry, cwd, SCROLLBACK, None, &[])
    }

    #[allow(clippy::too_many_arguments)]
    pub fn spawn_configured(
        id: u64,
        proxy: EventLoopProxy<UserEvent>,
        shell: &str,
        args: &[String],
        geometry: Geometry,
        cwd: Option<PathBuf>,
        scrollback: usize,
        socket_path: Option<&Path>,
        extra_env: &[(String, String)],
    ) -> std::io::Result<Self> {
        let listener = Listener {
            tab: id,
            proxy: proxy.clone(),
        };
        let term = Term::new(
            term_config_with(scrollback),
            &geometry.term_size(),
            listener,
        );
        let mut session = Session::new(term);
        session
            .graphics
            .set_cell(geometry.cell_width.into(), geometry.cell_height.into());
        let session = Arc::new(FairMutex::new(session));

        let mut env = HashMap::from([
            ("TERM".to_string(), "xterm-256color".to_string()),
            ("COLORTERM".to_string(), "truecolor".to_string()),
            ("TERM_PROGRAM".to_string(), "koral".to_string()),
        ]);
        if let Some(socket) = socket_path {
            env.insert(
                "KORAL_SOCKET".to_string(),
                socket.to_string_lossy().into_owned(),
            );
        }
        for (key, value) in extra_env {
            env.insert(key.clone(), value.clone());
        }
        let options = Options {
            shell: Some(Shell::new(shell.to_string(), args.to_vec())),
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
        let refresh_millihertz = Arc::new(AtomicU32::new(0));
        let reader_refresh = refresh_millihertz.clone();
        let reader_pty = pty.clone();
        let reader_session = session.clone();
        let (reader_wake, reader_closed) = (wake.clone(), closed.clone());
        std::thread::spawn(move || {
            read_loop(
                id,
                reader_pty,
                reader_session,
                proxy,
                reader_wake,
                reader_closed,
                reader_refresh,
            )
        });

        Ok(Self {
            id,
            session,
            projection: Mutex::new(RowProjection::default()),
            scroll: 0,
            title: None,
            rang: false,
            pty: Some(pty),
            writer,
            wake,
            closed,
            refresh_millihertz,
        })
    }

    pub fn set_refresh_millihertz(&self, refresh: Option<u32>) {
        self.refresh_millihertz
            .store(refresh.unwrap_or(0), Ordering::Relaxed);
    }

    pub fn write(&self, bytes: Vec<u8>) {
        let _ = self.writer.send(bytes);
    }

    pub fn resize(&mut self, geometry: Geometry) {
        let mut session = self.session.lock();
        session.term.resize(geometry.term_size());
        session.grid_changed();
        session
            .graphics
            .set_cell(geometry.cell_width.into(), geometry.cell_height.into());
        drop(session);
        if let Some(pty) = &self.pty {
            locked(pty).on_resize(geometry.window_size());
        }
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
        if let Some(pty) = self.pty.take() {
            // Off the window thread: `Pty::drop` calls `child.wait()` which can block.
            std::thread::spawn(move || drop(pty));
        }
    }
}

/// Drains the input channel into the PTY, and stops as soon as the tab is gone.
pub fn write_loop(mut output: File, inbox: Receiver<Vec<u8>>) {
    let poller = Poller::new().ok();
    if let Some(poller) = &poller {
        // SAFETY: `output` outlives the poller, both are dropped together in this thread.
        unsafe {
            let _ = poller.add(&output, PollEvent::writable(0));
        }
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
#[derive(Debug)]
pub struct Pacer {
    interval: Duration,
    painted: Option<Instant>,
    held: bool,
}

impl Default for Pacer {
    fn default() -> Self {
        Self::with_interval(refresh_interval(None))
    }
}

pub fn refresh_interval(refresh: Option<u32>) -> Duration {
    let rate = refresh
        .filter(|&rate| rate > 0)
        .unwrap_or(DEFAULT_REFRESH_MILLIHERTZ);
    Duration::from_nanos(1_000_000_000_000 / u64::from(rate))
}

impl Pacer {
    pub fn with_interval(interval: Duration) -> Self {
        Self {
            interval,
            painted: None,
            held: false,
        }
    }

    pub fn read(&mut self, now: Instant) -> bool {
        let due = self
            .painted
            .is_none_or(|at| now.duration_since(at) >= self.interval);
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

pub struct Burst<'a> {
    /// Contiguous slice of the staging buffer, in stream order.
    chunks: [&'a [u8]; 1],
    empty: bool,
    eof: bool,
    /// The staging cap was reached: more reads are pending, so the burst is still open.
    capped: bool,
}

impl<'a> Burst<'a> {
    pub fn chunks(&self) -> &[&'a [u8]] {
        if self.empty { &[] } else { &self.chunks }
    }

    pub fn is_empty(&self) -> bool {
        self.empty
    }

    pub fn eof(&self) -> bool {
        self.eof
    }

    pub fn capped(&self) -> bool {
        self.capped
    }
}

/// Reads every contiguous byte currently available into `stage`, growing it on demand.
/// The caller holds the PTY lock for the whole drain, so the child cannot interleave writes.
/// A closed tab drops the partial batch and stops at once.
pub fn drain<'a, R: Read>(
    reader: &mut R,
    stage: &'a mut Vec<u8>,
    closed: &AtomicBool,
) -> Burst<'a> {
    let mut used = 0;
    let (eof, capped) = loop {
        if closed.load(Ordering::SeqCst) {
            return Burst {
                chunks: [&[]],
                empty: true,
                eof: false,
                capped: false,
            };
        }
        if used == MAX_STAGE {
            break (false, true);
        }
        if used == stage.len() {
            let grown = (stage.len() * 2).clamp(64 * 1024, MAX_STAGE);
            stage.resize(grown, 0);
        }
        match reader.read(&mut stage[used..]) {
            Ok(0) => break (true, false),
            Ok(count) => used += count,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == ErrorKind::WouldBlock => break (false, false),
            Err(_) => break (true, false),
        }
    };
    Burst {
        chunks: [&stage[..used]],
        empty: used == 0,
        eof,
        capped,
    }
}

/// Drives the PTY, handing each chunk of output and the end of each burst to `sink`.
///
/// Returns the child's exit status when the child is seen to exit on its own.
pub fn pump(
    pty: &Mutex<Pty>,
    poller: &Arc<Poller>,
    closed: &AtomicBool,
    mut sink: impl FnMut(Option<&[&[u8]]>),
) -> Option<ExitStatus> {
    // Registers the master and the SIGCHLD pipe, so a wake means either side has news.
    // SAFETY: the PTY outlives the poller, which lives as long as the tab.
    unsafe {
        locked(pty)
            .register(poller, PollEvent::readable(0), PollMode::Level)
            .ok()?
    };

    let mut events = Events::new();
    let mut stage = Vec::new();
    loop {
        events.clear();
        if poller.wait(&mut events, None).is_err() || closed.load(Ordering::SeqCst) {
            return None;
        }
        // The child's death is authoritative: a job left behind can hold the slave open for good.
        let (exit, burst) = {
            let mut pty = locked(pty);
            let exit = pty
                .next_child_event()
                .map(|ChildEvent::Exited(status)| status);
            let burst = drain(pty.reader(), &mut stage, closed);
            (exit, burst)
        };
        if closed.load(Ordering::SeqCst) {
            return None;
        }
        if !burst.is_empty() {
            sink(Some(burst.chunks()));
        }
        if !burst.capped() || exit.is_some() {
            sink(None);
        }
        if burst.eof() {
            return exit.flatten();
        }
        if exit.is_some() {
            return exit.flatten();
        }
        if locked(pty)
            .reregister(poller, PollEvent::readable(0), PollMode::Level)
            .is_err()
        {
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
    refresh_millihertz: Arc<AtomicU32>,
) {
    let (to_parser, from_reader) = sync_channel::<Option<Vec<u8>>>(8);
    let (recycle_tx, recycle_rx) = channel::<Vec<u8>>();
    let parser = std::thread::spawn(move || {
        let mut pacer = Pacer::default();
        let mut previous_rate = 0;
        while let Ok(item) = from_reader.recv() {
            match item {
                Some(mut buf) => {
                    let now = Instant::now();
                    let mut session = session.lock();
                    session.feed(&buf, now);
                    for reply in session.replies.drain(..) {
                        let _ = proxy.send_event(UserEvent::Reply(id, reply));
                    }
                    for notice in session.notices.drain(..) {
                        let _ = proxy.send_event(UserEvent::Notify(id, notice));
                    }
                    for op in session.clipboard_ops.drain(..) {
                        let _ = proxy.send_event(UserEvent::ClipboardOp(id, op));
                    }
                    for mut event in session.events.drain(..) {
                        event.pane_id = id;
                        let _ = proxy.send_event(UserEvent::BlockEvent(id, event));
                    }
                    drop(session);
                    let rate = refresh_millihertz.load(Ordering::Relaxed);
                    if rate != previous_rate {
                        pacer.interval = refresh_interval(Some(rate));
                        previous_rate = rate;
                    }
                    if pacer.read(Instant::now()) {
                        let _ = proxy.send_event(UserEvent::Redraw);
                    }
                    buf.clear();
                    if buf.capacity() > MAX_STAGE {
                        buf.shrink_to(MAX_STAGE);
                    }
                    let _ = recycle_tx.send(buf);
                }
                None => {
                    if pacer.drained() {
                        let _ = proxy.send_event(UserEvent::Redraw);
                    }
                }
            }
        }
        let _ = proxy.send_event(UserEvent::Exited(id));
    });

    let _ = pump(&pty, &poller, &closed, |chunk| match chunk {
        Some(chunks) => {
            let total: usize = chunks.iter().map(|part| part.len()).sum();
            let mut buf = recycle_rx.try_recv().unwrap_or_default();
            buf.clear();
            buf.reserve(total);
            for part in chunks {
                buf.extend_from_slice(part);
            }
            let _ = to_parser.send(Some(buf));
        }
        None => {
            let _ = to_parser.send(None);
        }
    });
    drop(to_parser);
    let _ = parser.join();
}

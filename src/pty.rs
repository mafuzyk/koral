use std::collections::HashMap;
use std::fs::File;
use std::io::{ErrorKind, Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{Sender, channel};
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, EventListener, OnResize, WindowSize};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::tty::{self, Options, Pty, Shell};
use alacritty_terminal::vte::ansi::Rgb;
use polling::{Event as PollEvent, Events, Poller};
use winit::event_loop::EventLoopProxy;

use crate::session::Session;

const SCROLLBACK: usize = 10_000;
const FRAME: Duration = Duration::from_millis(16);

type ColorReply = Arc<dyn Fn(Rgb) -> String + Send + Sync>;

pub enum UserEvent {
    Redraw,
    Exited(u64),
    Reply(u64, String),
    Copy(String),
    Color(u64, usize, ColorReply),
    Colors,
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

    fn window_size(self) -> WindowSize {
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
            Event::PtyWrite(text) => UserEvent::Reply(self.tab, text),
            Event::ClipboardStore(_, text) => UserEvent::Copy(text),
            Event::ColorRequest(index, reply) => UserEvent::Color(self.tab, index, reply),
            _ => return,
        };
        let _ = self.proxy.send_event(event);
    }
}

pub struct Terminal {
    pub id: u64,
    pub session: Arc<FairMutex<Session<Listener>>>,
    pub scroll: usize,
    pty: Pty,
    writer: Sender<Vec<u8>>,
}

impl Terminal {
    pub fn spawn(
        id: u64,
        proxy: EventLoopProxy<UserEvent>,
        shell: &str,
        geometry: Geometry,
        cwd: Option<PathBuf>,
    ) -> std::io::Result<Self> {
        let config = Config { scrolling_history: SCROLLBACK, ..Config::default() };
        let listener = Listener { tab: id, proxy: proxy.clone() };
        let term = Term::new(config, &geometry.term_size(), listener);
        let session = Arc::new(FairMutex::new(Session::new(term)));

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
        let pty = tty::new(&options, geometry.window_size(), id)?;

        let (writer, inbox) = channel::<Vec<u8>>();
        let mut output = pty.file().try_clone()?;
        std::thread::spawn(move || {
            for bytes in inbox {
                write_all(&mut output, &bytes);
            }
        });

        let input = pty.file().try_clone()?;
        let reader_session = session.clone();
        std::thread::spawn(move || read_loop(id, input, reader_session, proxy));

        Ok(Self { id, session, scroll: 0, pty, writer })
    }

    pub fn write(&self, bytes: Vec<u8>) {
        let _ = self.writer.send(bytes);
    }

    pub fn resize(&mut self, geometry: Geometry) {
        self.session.lock().term.resize(geometry.term_size());
        self.pty.on_resize(geometry.window_size());
    }
}

// The PTY is non-blocking, and a slow child must never stall the window thread.
fn write_all(file: &mut File, mut bytes: &[u8]) {
    while !bytes.is_empty() {
        match file.write(bytes) {
            Ok(written) => bytes = &bytes[written..],
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
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

fn read_loop(
    id: u64,
    mut file: File,
    session: Arc<FairMutex<Session<Listener>>>,
    proxy: EventLoopProxy<UserEvent>,
) {
    let mut pacer = Pacer::default();

    let Ok(poller) = Poller::new() else {
        let _ = proxy.send_event(UserEvent::Exited(id));
        return;
    };
    // SAFETY: `file` outlives the poller, both are dropped when this function returns.
    if unsafe { poller.add(&file, PollEvent::readable(0)) }.is_err() {
        let _ = proxy.send_event(UserEvent::Exited(id));
        return;
    }

    let mut events = Events::new();
    let mut buffer = vec![0; 64 * 1024];
    loop {
        events.clear();
        if poller.wait(&mut events, None).is_err() {
            break;
        }
        loop {
            match file.read(&mut buffer) {
                Ok(0) => {
                    let _ = poller.delete(&file);
                    let _ = proxy.send_event(UserEvent::Exited(id));
                    return;
                }
                Ok(count) => {
                    session.lock().feed(&buffer[..count], Instant::now());
                    if pacer.read(Instant::now()) {
                        let _ = proxy.send_event(UserEvent::Redraw);
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == ErrorKind::Interrupted => {}
                Err(_) => {
                    let _ = poller.delete(&file);
                    let _ = proxy.send_event(UserEvent::Exited(id));
                    return;
                }
            }
        }
        if pacer.drained() {
            let _ = proxy.send_event(UserEvent::Redraw);
        }
        if poller.modify(&file, PollEvent::readable(0)).is_err() {
            break;
        }
    }
    let _ = proxy.send_event(UserEvent::Exited(id));
}

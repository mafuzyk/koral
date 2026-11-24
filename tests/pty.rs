use std::io::{self, ErrorKind, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use alacritty_terminal::event::WindowSize;
use alacritty_terminal::tty::{self, Options, Pty, Shell};
use koral::pty::{device_reply, drain, pump, term_config_with, write_loop};
use polling::Poller;

#[test]
fn a_device_query_answers_as_a_vt220() {
    assert_eq!(device_reply("\x1b[?6c".to_string()), "\x1b[?62;c");
}

#[test]
fn other_replies_pass_through() {
    assert_eq!(device_reply("\x1b[0n".to_string()), "\x1b[0n");
}

fn shell(command: &str) -> Pty {
    let options = Options {
        shell: Some(Shell::new(
            "/bin/sh".to_string(),
            vec!["-c".to_string(), command.to_string()],
        )),
        ..Options::default()
    };
    let size = WindowSize {
        num_lines: 24,
        num_cols: 80,
        cell_width: 8,
        cell_height: 16,
    };
    tty::new(&options, size, 7).expect("a pty with a shell")
}

/// The job survives the shell, so the master never reaches EOF: only the child exit can end the tab.
#[test]
fn the_shell_exit_ends_the_tab_while_a_background_job_lingers() {
    let pty = Arc::new(Mutex::new(shell("trap '' HUP; sleep 5 & exit 7")));
    let poller = Arc::new(Poller::new().expect("a poller"));
    let closed = AtomicBool::new(false);
    let (report, exit) = channel();
    let reader = pty.clone();
    let started = Instant::now();
    thread::spawn(move || {
        let status = pump(&reader, &poller, &closed, |_| {});
        let _ = report.send(status.and_then(|status| status.code()));
    });

    let code = exit
        .recv_timeout(Duration::from_secs(3))
        .expect("the shell exit must not wait for the background job");
    assert_eq!(code, Some(7));
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn a_blocked_writer_stops_when_the_tab_closes() {
    // Raw mode stops the line discipline from draining or echoing what nobody reads.
    let pty = shell("stty raw -echo; sleep 5");
    thread::sleep(Duration::from_millis(300));
    let mut output = pty.file().try_clone().expect("a master clone");
    let chunk = vec![b'x'; 4096];
    let mut filled = 0;
    while filled < 1 << 20 {
        match output.write(&chunk) {
            Ok(count) => filled += count,
            Err(error) if error.kind() == ErrorKind::WouldBlock => break,
            Err(error) => panic!("cannot fill the pty buffer: {error}"),
        }
    }
    assert!(
        filled > 0,
        "the pty buffer must fill before the writer blocks"
    );

    let (input, inbox) = channel();
    let (done, finished) = channel();
    thread::spawn(move || {
        write_loop(output, inbox);
        let _ = done.send(());
    });
    input
        .send(vec![b'y'; 1 << 20])
        .expect("hand a chunk to the writer");
    thread::sleep(Duration::from_millis(200));
    assert!(
        finished.try_recv().is_err(),
        "the writer must still be waiting on the pty"
    );
    drop(input);

    finished
        .recv_timeout(Duration::from_secs(2))
        .expect("the writer must drop the pending input once the tab is gone");
}

/// Closing a tab must release the reader, or the PTY outlives the tab and the shell is never hung up.
#[test]
fn a_closed_tab_releases_the_reader() {
    let pty = Arc::new(Mutex::new(shell("sleep 5")));
    let poller = Arc::new(Poller::new().expect("a poller"));
    let closed = Arc::new(AtomicBool::new(false));
    let (done, finished) = channel();
    let (reader, flag, wake) = (pty.clone(), closed.clone(), poller.clone());
    thread::spawn(move || {
        pump(&reader, &wake, &flag, |_| {});
        let _ = done.send(());
    });
    thread::sleep(Duration::from_millis(200));
    assert!(
        finished.try_recv().is_err(),
        "the reader must be waiting on the pty"
    );

    closed.store(true, Ordering::SeqCst);
    let _ = poller.notify();
    finished
        .recv_timeout(Duration::from_secs(2))
        .expect("the reader must stop once the tab is gone");
}

#[test]
fn configurable_scrollback_propagates_to_terminal_config() {
    assert_eq!(term_config_with(25_000).scrolling_history, 25_000);
    assert_eq!(term_config_with(0).scrolling_history, 0);
}

/// A reader whose scripted reads are all immediately available, then WouldBlock.
struct Scripted {
    reads: Vec<Vec<u8>>,
    next: usize,
    reads_done: usize,
    close_after: Option<usize>,
    closed: Arc<AtomicBool>,
}

impl Scripted {
    fn new(reads: &[&[u8]]) -> Self {
        Self {
            reads: reads.iter().map(|chunk| chunk.to_vec()).collect(),
            next: 0,
            reads_done: 0,
            close_after: None,
            closed: Arc::new(AtomicBool::new(false)),
        }
    }

    fn closing_after(mut self, count: usize) -> Self {
        self.close_after = Some(count);
        self
    }
}

impl Read for Scripted {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.reads_done += 1;
        if self.close_after == Some(self.reads_done) {
            self.closed.store(true, Ordering::SeqCst);
        }
        let Some(chunk) = self.reads.get(self.next) else {
            return Err(ErrorKind::WouldBlock.into());
        };
        self.next += 1;
        let count = chunk.len().min(buffer.len());
        buffer[..count].copy_from_slice(&chunk[..count]);
        Ok(count)
    }
}

#[test]
fn reaching_the_staging_cap_does_not_report_a_drain() {
    let full = vec![b'x'; 1024 * 1024];
    let mut stage = vec![0; full.len()];
    let mut reader = Scripted::new(&[&full, &full, b"tail"]);
    let closed = reader.closed.clone();
    for _ in 0..2 {
        let burst = drain(&mut reader, &mut stage, &closed);
        assert!(burst.capped());
        assert!(!burst.eof());
        assert_eq!(burst.chunks(), &[full.as_slice()]);
    }
    let burst = drain(&mut reader, &mut stage, &closed);
    assert!(!burst.capped());
    assert!(!burst.eof());
    assert_eq!(burst.chunks(), &[b"tail".as_slice()]);
}

#[test]
fn a_nonblocking_drain_hands_every_read_to_the_sink_in_one_batch() {
    let mut reader = Scripted::new(&[b"aa", b"bbb", b"cccc"]);
    let mut staging = Vec::new();
    let closed = reader.closed.clone();
    let burst = drain(&mut reader, &mut staging, &closed);

    assert!(!burst.eof());
    assert_eq!(burst.chunks(), &[b"aa".as_slice(), b"bbb", b"cccc"]);
    assert_eq!(
        reader.reads_done, 4,
        "the drain must run until the reader would block"
    );
}

#[test]
fn a_drain_keeps_bytes_from_successive_reads_in_order() {
    let mut reader = Scripted::new(&[b"head", b"-", b"tail"]);
    let mut staging = Vec::new();
    let closed = reader.closed.clone();
    let burst = drain(&mut reader, &mut staging, &closed);

    let joined: Vec<u8> = burst
        .chunks()
        .iter()
        .flat_map(|part| part.iter())
        .copied()
        .collect();
    assert_eq!(joined, b"head-tail");
}

#[test]
fn a_close_mid_drain_stops_before_the_next_read() {
    let mut reader = Scripted::new(&[b"first", b"second", b"third"]).closing_after(1);
    let mut staging = Vec::new();
    let closed = reader.closed.clone();
    let burst = drain(&mut reader, &mut staging, &closed);

    assert!(
        burst.is_empty(),
        "a closed tab must not publish a partial batch"
    );
    assert_eq!(
        reader.reads_done, 1,
        "the drain must stop immediately on close"
    );
}

#[test]
fn a_drain_returns_eof_with_the_bytes_read_before_it() {
    let mut reader = Scripted::new(&[b"last", b"", b"never"]);
    let mut staging = Vec::new();
    let closed = reader.closed.clone();
    let burst = drain(&mut reader, &mut staging, &closed);

    assert!(burst.eof());
    assert_eq!(burst.chunks(), &[b"last".as_slice()]);
    assert_eq!(reader.reads_done, 2, "the drain must stop at EOF");
}

#[test]
fn a_drain_never_stages_more_than_one_megabyte() {
    let chunk = vec![b'x'; 64 * 1024];
    let chunks: Vec<&[u8]> = vec![&chunk; 32];
    let mut reader = Scripted::new(&chunks);
    let mut staging = Vec::new();
    let closed = reader.closed.clone();
    let burst = drain(&mut reader, &mut staging, &closed);

    let total: usize = burst.chunks().iter().map(|part| part.len()).sum();
    assert_eq!(total, 1024 * 1024);
    assert_eq!(reader.reads_done, 16, "the drain must stop at the cap");
    assert!(staging.len() <= 1024 * 1024);
}

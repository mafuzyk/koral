use std::io::{ErrorKind, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use alacritty_terminal::event::WindowSize;
use alacritty_terminal::tty::{self, Options, Pty, Shell};
use koral::pty::{device_reply, pump, write_loop};
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
        shell: Some(Shell::new("/bin/sh".to_string(), vec!["-c".to_string(), command.to_string()])),
        ..Options::default()
    };
    let size = WindowSize { num_lines: 24, num_cols: 80, cell_width: 8, cell_height: 16 };
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
    assert!(filled > 0, "the pty buffer must fill before the writer blocks");

    let (input, inbox) = channel();
    let (done, finished) = channel();
    thread::spawn(move || {
        write_loop(output, inbox);
        let _ = done.send(());
    });
    input.send(vec![b'y'; 1 << 20]).expect("hand a chunk to the writer");
    thread::sleep(Duration::from_millis(200));
    assert!(finished.try_recv().is_err(), "the writer must still be waiting on the pty");
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
    assert!(finished.try_recv().is_err(), "the reader must be waiting on the pty");

    closed.store(true, Ordering::SeqCst);
    let _ = poller.notify();
    finished
        .recv_timeout(Duration::from_secs(2))
        .expect("the reader must stop once the tab is gone");
}

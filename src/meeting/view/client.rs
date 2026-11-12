//! A reconnecting `meeting.sock` reader for viewers.
//!
//! Runs on its own thread and hands lines over a channel, so a viewer's
//! loop never blocks on the socket. When the daemon goes away it retries
//! with a short backoff; every new connection starts with the daemon's
//! `hello` snapshot, which is all a viewer needs to catch up.

use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

/// What the reader reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Feed {
    Connected,
    /// One NDJSON line, without its newline.
    Line(String),
    Disconnected,
}

/// Handle to a running reader.
pub struct FeedHandle {
    stop: Arc<AtomicBool>,
    resync: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FeedHandle {
    /// Drop the current connection and reconnect, for a fresh `hello`.
    pub fn resync(&self) {
        self.resync.store(true, Ordering::SeqCst);
    }

    /// Stop reading and wait for the thread.
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for FeedHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

const POLL: Duration = Duration::from_millis(200);
const MAX_BACKOFF: Duration = Duration::from_secs(2);

/// Start reading `socket` until stopped or `tx` is dropped.
pub fn spawn_feed(socket: PathBuf, tx: Sender<Feed>) -> FeedHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let resync = Arc::new(AtomicBool::new(false));
    let (s, r) = (stop.clone(), resync.clone());
    let thread = std::thread::spawn(move || run(socket, tx, s, r));
    FeedHandle {
        stop,
        resync,
        thread: Some(thread),
    }
}

fn run(socket: PathBuf, tx: Sender<Feed>, stop: Arc<AtomicBool>, resync: Arc<AtomicBool>) {
    let mut backoff = Duration::from_millis(100);
    let mut announced_down = false;
    while !stop.load(Ordering::SeqCst) {
        let stream = match UnixStream::connect(&socket) {
            Ok(s) => s,
            Err(_) => {
                if !announced_down {
                    announced_down = true;
                    if tx.send(Feed::Disconnected).is_err() {
                        return;
                    }
                }
                sleep_unless_stopped(backoff, &stop);
                backoff = (backoff * 2).min(MAX_BACKOFF);
                continue;
            }
        };
        backoff = Duration::from_millis(100);
        resync.store(false, Ordering::SeqCst);
        let _ = stream.set_read_timeout(Some(POLL));
        if tx.send(Feed::Connected).is_err() {
            return;
        }
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        loop {
            if stop.load(Ordering::SeqCst) {
                return;
            }
            if resync.swap(false, Ordering::SeqCst) {
                break;
            }
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if line.ends_with('\n') {
                        let text = line.trim_end_matches(['\n', '\r']).to_string();
                        line.clear();
                        if !text.is_empty() && tx.send(Feed::Line(text)).is_err() {
                            return;
                        }
                    }
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    // A partial line stays in `line` until the rest arrives.
                    continue;
                }
                Err(_) => break,
            }
        }
        announced_down = true;
        if tx.send(Feed::Disconnected).is_err() {
            return;
        }
    }
}

fn sleep_unless_stopped(d: Duration, stop: &AtomicBool) {
    let mut left = d;
    while !left.is_zero() && !stop.load(Ordering::SeqCst) {
        let step = left.min(Duration::from_millis(50));
        std::thread::sleep(step);
        left -= step;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::net::UnixListener;
    use std::sync::mpsc;

    fn recv(rx: &mpsc::Receiver<Feed>) -> Feed {
        rx.recv_timeout(Duration::from_secs(5)).expect("feed event")
    }

    #[test]
    fn reads_lines_and_reconnects_after_the_daemon_restarts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meeting.sock");
        let (tx, rx) = mpsc::channel();
        let feed = spawn_feed(path.clone(), tx);

        // Nothing listening yet.
        assert_eq!(recv(&rx), Feed::Disconnected);

        let listener = UnixListener::bind(&path).unwrap();
        let (mut conn, _) = listener.accept().unwrap();
        assert_eq!(recv(&rx), Feed::Connected);
        // A line split across two writes arrives whole.
        conn.write_all(br#"{"event":"hello","st"#).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        conn.write_all(b"atus\":\"idle\"}\n{\"event\":\"status\",\"status\":\"paused\"}\n")
            .unwrap();
        assert_eq!(
            recv(&rx),
            Feed::Line(r#"{"event":"hello","status":"idle"}"#.into())
        );
        assert_eq!(
            recv(&rx),
            Feed::Line(r#"{"event":"status","status":"paused"}"#.into())
        );

        // The daemon goes away and comes back.
        drop(conn);
        drop(listener);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(recv(&rx), Feed::Disconnected);
        let listener = UnixListener::bind(&path).unwrap();
        let (mut conn, _) = listener.accept().unwrap();
        assert_eq!(recv(&rx), Feed::Connected);
        conn.write_all(b"{\"event\":\"hello\",\"status\":\"recording\"}\n")
            .unwrap();
        assert!(matches!(recv(&rx), Feed::Line(l) if l.contains("recording")));

        // A resync drops the connection and makes a new one.
        feed.resync();
        assert_eq!(recv(&rx), Feed::Disconnected);
        let (_conn2, _) = listener.accept().unwrap();
        assert_eq!(recv(&rx), Feed::Connected);
        feed.stop();
    }
}

//! The PTY / terminal manager.
//!
//! Spawns a CLI (e.g. `codex`) in a real pseudo-terminal — the actual binary, not a wrapper.
//! Output is chunked, each chunk tagged with a monotonic `seq`, and kept in a bounded ring
//! buffer so a UI node can reattach after a reload without dropping or duplicating a line:
//! ask for a [`snapshot`](TerminalManager::snapshot), then ignore live chunks whose `seq` is
//! already in it.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use portable_pty::{CommandBuilder, MasterPty, NativePtySystem, PtySize, PtySystem};
use serde::Serialize;

/// One chunk of terminal output. `data` is raw bytes — the caller (xterm.js) owns decoding,
/// so a multibyte char split across a read boundary is never corrupted.
#[derive(Clone, Serialize)]
pub struct Output {
    pub seq: u64,
    pub data: Vec<u8>,
}

// Ring buffer capped by chunk count, not bytes. Enough for reattach-after-reload;
// switch to a byte budget if someone wants real scrollback history.
const MAX_CHUNKS: usize = 5000;

struct Terminal {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    buffer: Arc<Mutex<VecDeque<Output>>>,
}

/// A callback invoked once per output chunk, from the reader thread. Identra's Tauri layer
/// passes a closure that emits a window event; a test passes an mpsc sender.
type Sink = Arc<dyn Fn(String, Output) + Send + Sync>;

pub struct TerminalManager {
    terminals: Mutex<HashMap<String, Terminal>>,
    sink: Sink,
}

impl TerminalManager {
    pub fn new(sink: Sink) -> Self {
        Self {
            terminals: Mutex::new(HashMap::new()),
            sink,
        }
    }

    /// Spawn `cmd args` in a PTY under `id`. Reusing an existing `id` kills the old one first.
    ///
    /// `env` sets extra process env on top of the inherited environment (the builder does not clear
    /// it, so the CLI still finds the user's login). Identra uses it to hand each node its own bus
    /// bearer without the token ever passing through the frontend.
    // Every argument here is a distinct PTY spawn knob (command, args, cwd, env, size). Folding
    // them into a struct would add a type for two call sites without making either clearer.
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        &self,
        id: String,
        cmd: &str,
        args: &[String],
        cwd: Option<&str>,
        env: &[(String, String)],
        rows: u16,
        cols: u16,
    ) -> Result<(), Error> {
        self.kill(&id)?; // idempotent restart

        let pty = NativePtySystem::default()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(Error::pty)?;

        let mut builder = CommandBuilder::new(cmd);
        builder.args(args);
        for (key, value) in env {
            builder.env(key, value);
        }
        if let Some(dir) = cwd {
            builder.cwd(dir);
        }
        let child = pty.slave.spawn_command(builder).map_err(Error::pty)?;
        drop(pty.slave); // let the reader see EOF when the child exits

        let writer = pty.master.take_writer().map_err(Error::pty)?;
        let mut reader = pty.master.try_clone_reader().map_err(Error::pty)?;

        let buffer = Arc::new(Mutex::new(VecDeque::new()));
        let seq = Arc::new(AtomicU64::new(0));
        let (sink, buf, id_for_thread) = (self.sink.clone(), buffer.clone(), id.clone());

        thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            loop {
                match reader.read(&mut chunk) {
                    Ok(0) | Err(_) => break, // EOF or the pty went away
                    Ok(n) => {
                        let out = Output {
                            seq: seq.fetch_add(1, Ordering::SeqCst) + 1,
                            data: chunk[..n].to_vec(),
                        };
                        {
                            let mut b = buf.lock().unwrap();
                            b.push_back(out.clone());
                            while b.len() > MAX_CHUNKS {
                                b.pop_front();
                            }
                        }
                        (sink)(id_for_thread.clone(), out);
                    }
                }
            }
        });

        self.terminals.lock().unwrap().insert(
            id,
            Terminal {
                master: pty.master,
                writer,
                child,
                buffer,
            },
        );
        Ok(())
    }

    /// Send keystrokes / bytes to the terminal's stdin.
    pub fn input(&self, id: &str, data: &[u8]) -> Result<(), Error> {
        let mut terms = self.terminals.lock().unwrap();
        let term = terms.get_mut(id).ok_or(Error::NotFound)?;
        term.writer.write_all(data)?;
        term.writer.flush()?;
        Ok(())
    }

    /// Resize the PTY so the child re-wraps its output to match the on-screen terminal.
    pub fn resize(&self, id: &str, rows: u16, cols: u16) -> Result<(), Error> {
        let terms = self.terminals.lock().unwrap();
        let term = terms.get(id).ok_or(Error::NotFound)?;
        term.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(Error::pty)?;
        Ok(())
    }

    /// Everything still in the ring buffer, concatenated, plus the last `seq` it covers.
    /// Write the bytes to a fresh xterm on reattach, then drop live chunks with `seq <= last`.
    pub fn snapshot(&self, id: &str) -> Option<(Vec<u8>, u64)> {
        let terms = self.terminals.lock().unwrap();
        let buf = terms.get(id)?.buffer.lock().unwrap();
        let mut data = Vec::new();
        let mut last = 0;
        for out in buf.iter() {
            data.extend_from_slice(&out.data);
            last = out.seq;
        }
        Some((data, last))
    }

    /// Kill the child and forget the terminal. No-op if `id` is unknown.
    pub fn kill(&self, id: &str) -> Result<(), Error> {
        if let Some(mut term) = self.terminals.lock().unwrap().remove(id) {
            let _ = term.child.kill();
        }
        Ok(())
    }

    /// Ids of every live terminal.
    pub fn ids(&self) -> Vec<String> {
        self.terminals.lock().unwrap().keys().cloned().collect()
    }
}

#[derive(Debug)]
pub enum Error {
    NotFound,
    Io(std::io::Error),
    Pty(String),
}

impl Error {
    fn pty(e: impl fmt::Display) -> Self {
        Error::Pty(e.to_string())
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotFound => write!(f, "terminal not found"),
            Error::Io(e) => write!(f, "io error: {e}"),
            Error::Pty(e) => write!(f, "pty error: {e}"),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn spawns_captures_and_replays_output() {
        let (tx, rx) = mpsc::channel();
        let mgr = TerminalManager::new(Arc::new(move |_id, out: Output| {
            let _ = tx.send(out);
        }));

        mgr.start(
            "t1".into(),
            "echo",
            &["hello-identra".into()],
            None,
            &[],
            24,
            80,
        )
        .expect("spawn echo");

        // Live sink delivers the bytes.
        let mut live = Vec::new();
        while let Ok(out) = rx.recv_timeout(Duration::from_secs(5)) {
            live.extend_from_slice(&out.data);
            if String::from_utf8_lossy(&live).contains("hello-identra") {
                break;
            }
        }
        assert!(String::from_utf8_lossy(&live).contains("hello-identra"));

        // Snapshot replays the same bytes and reports a non-zero seq (reattach path).
        let (snap, last) = mgr.snapshot("t1").expect("snapshot exists");
        assert!(String::from_utf8_lossy(&snap).contains("hello-identra"));
        assert!(last >= 1, "seq should advance past zero");

        assert_eq!(
            mgr.input("nope", b"x")
                .map_err(|e| e.to_string())
                .unwrap_err(),
            "terminal not found"
        );
    }
}

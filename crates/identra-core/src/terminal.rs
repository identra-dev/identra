//! The PTY / terminal manager.
//!
//! Spawns a CLI (e.g. `codex`) in a real pseudo-terminal: the actual binary, not a wrapper.
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

/// One chunk of terminal output. `data` is raw bytes: the caller (xterm.js) owns decoding,
/// so a multibyte char split across a read boundary is never corrupted.
#[derive(Clone, Serialize)]
pub struct Output {
    pub seq: u64,
    pub data: Vec<u8>,
}

// Ring buffer capped by chunk count, not bytes. Enough for reattach-after-reload;
// switch to a byte budget if someone wants real scrollback history.
const MAX_CHUNKS: usize = 5000;

/// The child is shared with the reader thread rather than owned here, because both need it: this
/// side kills it, and the reader waits on it after EOF to learn how it ended.
type Child = Arc<Mutex<Box<dyn portable_pty::Child + Send + Sync>>>;

/// How long a terminal must be silent before I call it idle rather than working.
///
/// This is a heuristic and it is the honest one available. A CLI agent gives no "I am done" signal
/// over a PTY: it just stops printing. So thinking looks like output (a spinner, a token stream)
/// and finished looks like quiet. A second and a half is long enough not to trip on the gap between
/// two tokens, short enough that a waiting agent is not left guessing.
const QUIET: std::time::Duration = std::time::Duration::from_millis(1500);

struct Terminal {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Child,
    buffer: Arc<Mutex<VecDeque<Output>>>,
    /// When this node last printed anything. Shared with the reader thread, which is what stamps it.
    last_output: Arc<Mutex<std::time::Instant>>,
    exited: Arc<std::sync::atomic::AtomicBool>,
}

/// What a node is doing, as far as anything outside it can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Printing, so it is working on something.
    Running,
    /// Alive and quiet, and the last thing it printed does not read like a question. Either sitting
    /// at its prompt or thinking without saying anything.
    Idle,
    /// Alive, quiet, and the last thing it printed reads like it is waiting for an answer.
    #[serde(rename = "needs-input")]
    NeedsInput,
    /// The agent is gone.
    Exited,
}

/// How far back to read when deciding whether a quiet node is waiting on its human. A prompt is the
/// last thing on screen by definition, so this only has to be big enough to cover the final line
/// plus whatever escape noise is wrapped around it.
const PROMPT_TAIL_BYTES: usize = 2048;

/// Does the tail of a node's output read like the agent asked something and stopped?
///
/// This is a guess and it is worth being honest about why it has to be. A CLI over a PTY has no
/// channel to say "I am waiting for you", so the only evidence is what it printed. I take the last
/// line that has anything on it and look for the two shapes that a CLI waiting on a human almost
/// always has: it ends in a question mark, or it offers a choice like `(y/N)`.
///
/// The caller only asks once the node has already gone quiet, and that precondition does most of
/// the work: prose that happens to end in a question mark mid-stream is still Running, so it never
/// reaches here. What is left is a node that printed a question and then stopped, which is the
/// thing being looked for.
///
/// Getting it wrong is cheap in one direction and free in the other. A false positive shows an
/// aubergine ring on a node that was merely idle. A false negative just leaves it idle, which is
/// what it would have been anyway. So I would rather it be a little eager than miss a real prompt,
/// but not so eager that every node on the canvas glows.
fn looks_like_a_prompt(text: &str) -> bool {
    let Some(line) = text.lines().rev().find(|l| !l.trim().is_empty()) else {
        return false;
    };
    let line = line.trim_end();
    // A trailing cursor block or box-drawing edge is common in a TUI prompt, and it sits after the
    // punctuation that matters, so it has to come off before the question mark is checked for.
    let line = line.trim_end_matches(|c: char| {
        c.is_whitespace() || matches!(c, '\u{2502}' | '\u{2588}' | '_' | '|')
    });
    if line.ends_with('?') {
        return true;
    }
    // A y/n affordance, in the spellings CLIs actually use. Lower-cased once so `(Y/n)`, the
    // default-yes form, is not a separate case to remember.
    let lowered = line.to_lowercase();
    ["(y/n)", "[y/n]", "(yes/no)", "[yes/no]", "(y)es", "y/n:"]
        .iter()
        .any(|shape| lowered.contains(shape))
}

/// Something that happened to a terminal, in the order it happened.
///
/// Exit is here rather than being left for the caller to notice because nobody else can see it: the
/// reader thread is the only thing holding the read end, so EOF is the first and only moment we
/// know the agent is gone. Without this a finished agent looks exactly like an idle one, and the UI
/// cannot tell "done" from "thinking".
#[derive(Clone)]
pub enum Event {
    Output(Output),
    /// The child is gone. `code` is `None` when it was killed by a signal or the status could not
    /// be read, which is a real state and not worth inventing a number for.
    Exit {
        code: Option<u32>,
    },
}

/// A callback invoked once per event, from the reader thread. Identra's Tauri layer passes a
/// closure that emits a window event; a test passes an mpsc sender.
type Sink = Arc<dyn Fn(String, Event) + Send + Sync>;

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

        let child: Child = Arc::new(Mutex::new(child));
        let buffer = Arc::new(Mutex::new(VecDeque::new()));
        let seq = Arc::new(AtomicU64::new(0));
        let last_output = Arc::new(Mutex::new(std::time::Instant::now()));
        let exited = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (sink, buf, id_for_thread) = (self.sink.clone(), buffer.clone(), id.clone());
        let child_for_thread = child.clone();
        let (seen, done) = (last_output.clone(), exited.clone());

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
                        *seen.lock().unwrap() = std::time::Instant::now();
                        (sink)(id_for_thread.clone(), Event::Output(out));
                    }
                }
            }
            done.store(true, Ordering::SeqCst);
            // EOF means the child let go of the pty, so it is finished or nearly so. I wait here to
            // turn that into an exit code, then say so once. This thread is the only place that can:
            // it is the one holding the read end.
            let code = child_for_thread
                .lock()
                .unwrap()
                .wait()
                .ok()
                .map(|status| status.exit_code());
            (sink)(id_for_thread, Event::Exit { code });
        });

        self.terminals.lock().unwrap().insert(
            id,
            Terminal {
                master: pty.master,
                writer,
                child,
                buffer,
                last_output,
                exited,
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
    ///
    /// The reader thread still sees the resulting EOF and reports the exit, so a killed node ends
    /// up in the same visible state as one that quit on its own.
    pub fn kill(&self, id: &str) -> Result<(), Error> {
        if let Some(term) = self.terminals.lock().unwrap().remove(id) {
            let _ = term.child.lock().unwrap().kill();
        }
        Ok(())
    }

    /// Ids of every live terminal.
    pub fn ids(&self) -> Vec<String> {
        self.terminals.lock().unwrap().keys().cloned().collect()
    }

    /// The process this terminal spawned, so a caller can look at what it is actually doing.
    /// [`crate::session`] uses it to find the agent's conversation.
    pub fn pid(&self, id: &str) -> Option<u32> {
        self.terminals
            .lock()
            .unwrap()
            .get(id)?
            .child
            .lock()
            .unwrap()
            .process_id()
    }

    /// What `id` is doing, or `None` if there is no such terminal.
    ///
    /// Read from output timing rather than asked of the agent, because there is nothing to ask: a
    /// CLI over a PTY has no way to say "I finished". Quiet is the only signal it gives, so quiet is
    /// what I report, and the caller is told plainly that is what this means.
    ///
    /// The one refinement is that quiet has two meanings, and they are the two a person most wants
    /// told apart: an agent that is done, and an agent that asked something and is waiting. Only
    /// once a node has gone quiet do I read the tail of what it printed to separate them, so the
    /// text scan costs nothing while the node is actually working.
    pub fn status(&self, id: &str) -> Option<Status> {
        let terms = self.terminals.lock().unwrap();
        let term = terms.get(id)?;
        if term.exited.load(Ordering::SeqCst) {
            return Some(Status::Exited);
        }
        let quiet_for = term.last_output.lock().unwrap().elapsed();
        if quiet_for < QUIET {
            return Some(Status::Running);
        }
        // Only the last chunks can hold the final line, so I walk back from the end and stop as
        // soon as I have enough. The whole ring is up to 5000 chunks and reading it here would put
        // a scan of the entire scrollback behind a call the UI makes on every settle.
        let buf = term.buffer.lock().unwrap();
        let mut recent: Vec<u8> = Vec::new();
        for out in buf.iter().rev() {
            if recent.len() >= PROMPT_TAIL_BYTES {
                break;
            }
            recent.splice(0..0, out.data.iter().copied());
        }
        drop(buf);
        let text = crate::text::tail(&crate::text::strip_ansi(&recent), PROMPT_TAIL_BYTES);
        Some(if looks_like_a_prompt(&text) {
            Status::NeedsInput
        } else {
            Status::Idle
        })
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
    fn spawns_captures_replays_output_and_reports_the_exit() {
        let (tx, rx) = mpsc::channel();
        let mgr = TerminalManager::new(Arc::new(move |_id, event: Event| {
            let _ = tx.send(event);
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

        // Live sink delivers the bytes, and then says the agent is gone. `echo` exits on its own,
        // so this drives the whole life of a node: output, then exit, in that order.
        let mut live = Vec::new();
        let mut exit = None;
        while let Ok(event) = rx.recv_timeout(Duration::from_secs(5)) {
            match event {
                Event::Output(out) => live.extend_from_slice(&out.data),
                Event::Exit { code } => {
                    exit = Some(code);
                    break;
                }
            }
        }
        assert!(String::from_utf8_lossy(&live).contains("hello-identra"));
        // A finished agent has to be distinguishable from a quiet one, or the node cannot show it.
        assert_eq!(
            exit,
            Some(Some(0)),
            "echo exits cleanly and we hear about it"
        );

        // The same fact is readable on demand, which is what an agent waiting on a helper needs.
        // Exited is known, not inferred from silence, so waiting on it cannot be wrong.
        assert_eq!(mgr.status("t1"), Some(Status::Exited));
        assert_eq!(mgr.status("never-started"), None);

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

    /// The heuristic is checked on its own below. This checks the wiring around it: a real child in
    /// a real PTY that asks something and then waits has to come back as NeedsInput, and one that
    /// says something ordinary and waits has to stay Idle. Those two go through the ring buffer, the
    /// ANSI strip, and the quiet timer, none of which the pure test touches.
    #[test]
    fn a_waiting_agent_is_told_apart_from_a_quiet_one() {
        let mgr = TerminalManager::new(Arc::new(|_id, _event| {}));

        // `read` holds the child open with nothing more to print, which is exactly the shape of a
        // CLI sitting on a prompt: the question is the last thing in the buffer and it stays there.
        mgr.start(
            "asking".into(),
            "sh",
            &[
                "-c".into(),
                "printf 'Overwrite main.rs? (y/N) '; read x".into(),
            ],
            None,
            &[],
            24,
            80,
        )
        .expect("spawn a waiting child");

        mgr.start(
            "working".into(),
            "sh",
            &["-c".into(), "printf 'Wrote 42 lines.\\n'; read x".into()],
            None,
            &[],
            24,
            80,
        )
        .expect("spawn a quiet child");

        // Both are Running while the output is fresh, whatever they printed. The split only exists
        // once a node has actually gone quiet, so before QUIET there is nothing to tell apart.
        assert_eq!(mgr.status("asking"), Some(Status::Running));

        // Past the quiet threshold, with a margin so a slow machine does not make this flaky.
        std::thread::sleep(QUIET + Duration::from_millis(400));

        assert_eq!(
            mgr.status("asking"),
            Some(Status::NeedsInput),
            "a child sitting on a question is waiting on its human"
        );
        assert_eq!(
            mgr.status("working"),
            Some(Status::Idle),
            "a child that reported and stopped is just quiet"
        );

        let _ = mgr.kill("asking");
        let _ = mgr.kill("working");
    }

    /// The prompt heuristic, on its own, because it is the part with judgement in it and the part
    /// that would rot silently. Everything here is output I have actually watched these CLIs print.
    #[test]
    fn a_question_reads_differently_from_a_finished_thought() {
        // The shapes that mean someone is waiting on me.
        assert!(looks_like_a_prompt("Do you want to proceed?"));
        assert!(looks_like_a_prompt("Overwrite src/main.rs? (y/N)"));
        assert!(looks_like_a_prompt("Apply this patch [Y/n]"));
        assert!(looks_like_a_prompt("Continue? (yes/no)"));
        // Trailing blank lines are normal after a prompt and must not hide it.
        assert!(looks_like_a_prompt("Which file should I edit?\n\n  \n"));
        // A TUI draws a cursor or a box edge after the text. The question mark is still the signal.
        assert!(looks_like_a_prompt("Ready to continue? \u{2588}"));
        assert!(looks_like_a_prompt("Shall I run the tests? \u{2502}"));

        // The shapes that do not. An agent narrating its work is not asking me anything, even when
        // it says the word question, and a finished summary is the common case that must stay quiet.
        assert!(!looks_like_a_prompt("Done. 3 files changed."));
        assert!(!looks_like_a_prompt(
            "I considered the question of caching."
        ));
        assert!(!looks_like_a_prompt(""));
        assert!(!looks_like_a_prompt("   \n\n  "));
        // The last line is what counts: an earlier question that has since been answered and moved
        // past must not pin the node in needs-input forever.
        assert!(!looks_like_a_prompt(
            "Overwrite the file? (y/N)\ny\nWrote 42 lines."
        ));
    }
}

//! Finding the conversation an agent is having, so it survives closing the app.
//!
//! Without this, quitting Identra throws away every session: the node comes back, the agent starts
//! from nothing, and the user re-explains the project. The agents themselves already solve this,
//! they each have a `--resume` that takes a session id, but none of them will tell you what that id
//! is. There is no flag, no file they announce, no signal on the terminal.
//!
//! So I read it off the process. Every one of these CLIs keeps its transcript open as a `.jsonl`
//! whose filename is the session id, so the agent's own open file descriptors are the answer:
//! find the agent under the node's PTY, look at what it has open, and the id is sitting there in a
//! path. It needs no cooperation from the agent, and it cannot go stale, because it is read from
//! the live process rather than remembered.
//!
//! Reading the process is per OS, so it is three small functions behind one shape. Linux has
//! `/proc`, which is the same information for free: no subprocess, and this runs on a timer. macOS
//! has no `/proc`, so there it costs `pgrep`, `ps` and `lsof`. Same answers, and the price is why
//! the sampler backs off rather than polling hard.
//!
//! The one rule everywhere here: never guess. If two transcripts are open, or the process is gone
//! mid-read, I return nothing. A wrong session id resumes the wrong conversation, which is worse
//! than starting fresh, and the user cannot tell it happened.

use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;

/// A conversation an agent is having, and where it lives.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Session {
    /// The agent id, so a resumed node relaunches the CLI that owns this transcript.
    pub agent: String,
    pub id: String,
    /// The transcript itself. I keep it to check the session still exists before resuming, since a
    /// deleted transcript means a resume that fails on launch for no visible reason.
    pub file: PathBuf,
}

/// Which agents keep a resumable transcript, and how to ask them to resume one.
///
/// This is a table rather than a match because the interesting part is per agent data, and because
/// an agent whose resume flag I do not know is one I must not invent a flag for.
struct Resumable {
    agent: &'static str,
    /// Binary names, matched against the basename of a process's argv0.
    bins: &'static [&'static str],
    /// Args that resume a session, with the id appended last.
    resume: &'static [&'static str],
}

const RESUMABLE: &[Resumable] = &[
    Resumable {
        agent: "claude",
        bins: &["claude"],
        resume: &["--resume"],
    },
    Resumable {
        agent: "codex",
        bins: &["codex"],
        resume: &["resume"],
    },
];

/// The args that resume `session`, or `None` for an agent with no resume I know of.
///
/// The id is checked before it goes anywhere near a command line. It came out of a filename on
/// disk, and a filename is not a thing I control, so it is validated rather than trusted.
pub fn resume_args(session: &Session) -> Option<Vec<String>> {
    if !valid_id(&session.id) {
        return None;
    }
    let row = RESUMABLE.iter().find(|r| r.agent == session.agent)?;
    let mut args: Vec<String> = row.resume.iter().map(|s| (*s).to_string()).collect();
    args.push(session.id.clone());
    Some(args)
}

/// A session id is a filename stem the agent chose. I accept the shape they all use and nothing
/// else: anything with a space, a slash, or a dash-leading flag shape does not become an argument.
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && !id.starts_with('-')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

/// The agent this transcript belongs to, judged by where it lives.
///
/// Each CLI writes under its own directory, so the path says whose it is. I check the directory
/// rather than the filename because the filenames are all just uuids.
fn agent_for_transcript(path: &Path) -> Option<&'static str> {
    let text = path.to_str()?;
    if text.contains("/.claude/projects/") {
        return Some("claude");
    }
    if text.contains("/.codex/sessions/") {
        return Some("codex");
    }
    None
}

/// Direct children of `pid`, from `/proc`.
#[cfg(target_os = "linux")]
fn children(pid: u32) -> Vec<u32> {
    let mut out = Vec::new();
    let Ok(tasks) = std::fs::read_dir(format!("/proc/{pid}/task")) else {
        return out;
    };
    for task in tasks.flatten() {
        let path = task.path().join("children");
        if let Ok(text) = std::fs::read_to_string(path) {
            out.extend(
                text.split_whitespace()
                    .filter_map(|p| p.parse::<u32>().ok()),
            );
        }
    }
    out
}

/// Direct children of `pid`. macOS has no `/proc`, so this is what `pgrep` is for.
#[cfg(target_os = "macos")]
fn children(pid: u32) -> Vec<u32> {
    // pgrep exits non-zero when there are simply no children, which is the common case and not an
    // error, so the status is ignored and the empty output speaks for itself.
    let Ok(out) = Command::new("pgrep")
        .arg("-P")
        .arg(pid.to_string())
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .filter_map(|p| p.parse::<u32>().ok())
        .collect()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn children(_pid: u32) -> Vec<u32> {
    Vec::new()
}

/// The basename of what `pid` is running.
#[cfg(target_os = "linux")]
fn argv0(pid: u32) -> Option<String> {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let first = raw.split(|b| *b == 0).next()?;
    let text = std::str::from_utf8(first).ok()?;
    Path::new(text)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
}

/// What `pid` is running. `comm` is already the basename on macOS, which is what I compare.
#[cfg(target_os = "macos")]
fn argv0(pid: u32) -> Option<String> {
    let out = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() {
        return None;
    }
    Path::new(&text)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn argv0(_pid: u32) -> Option<String> {
    None
}

/// Transcripts `pid` currently has open.
///
/// Reading a live process's fds races it exiting, so every failure here is "no answer" rather than
/// an error: the process being gone is the normal case, not a problem.
#[cfg(target_os = "linux")]
fn open_transcripts(pid: u32) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(fds) = std::fs::read_dir(format!("/proc/{pid}/fd")) else {
        return out;
    };
    for fd in fds.flatten() {
        let Ok(target) = std::fs::read_link(fd.path()) else {
            continue;
        };
        if target.extension().is_some_and(|e| e == "jsonl")
            && agent_for_transcript(&target).is_some()
        {
            out.push(target);
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Transcripts `pid` has open, via `lsof`, since macOS cannot show them any cheaper.
///
/// `-Fn` asks for machine readable output: one field per line, each tagged by its first character,
/// so the paths are the lines starting with `n`. It beats parsing lsof's columns, which are aligned
/// for people and break on a path with a space in it.
#[cfg(target_os = "macos")]
fn open_transcripts(pid: u32) -> Vec<PathBuf> {
    let Ok(out) = Command::new("lsof")
        .args(["-p", &pid.to_string(), "-Fn"])
        .output()
    else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| line.strip_prefix('n'))
        .map(PathBuf::from)
        .filter(|p| {
            p.extension().is_some_and(|e| e == "jsonl") && agent_for_transcript(p).is_some()
        })
        .collect();
    found.sort();
    found.dedup();
    found
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn open_transcripts(_pid: u32) -> Vec<PathBuf> {
    Vec::new()
}

/// Every process under `root`, `root` included. A CLI is usually a wrapper script over a real
/// binary, so the agent is rarely the PTY's direct child.
fn descendants(root: u32) -> Vec<u32> {
    let mut found = vec![root];
    let mut queue = vec![root];
    // Depth is bounded by the real process tree, but I cap the walk anyway: this reads live state,
    // and a cycle in what I read would otherwise hang the sampler.
    let mut budget = 64;
    while let Some(pid) = queue.pop() {
        if budget == 0 {
            break;
        }
        budget -= 1;
        for child in children(pid) {
            if !found.contains(&child) {
                found.push(child);
                queue.push(child);
            }
        }
    }
    found
}

/// The session the agent under `pty_pid` is having, if it can be known for certain.
pub fn detect(pty_pid: u32) -> Option<Session> {
    for pid in descendants(pty_pid) {
        let Some(bin) = argv0(pid) else { continue };
        let Some(row) = RESUMABLE.iter().find(|r| r.bins.iter().any(|b| *b == bin)) else {
            continue;
        };
        let open = open_transcripts(pid);
        // Exactly one, or nothing. Two open transcripts means I cannot tell which conversation this
        // is, and picking one would resume the wrong one for a user who has no way to notice.
        if open.len() != 1 {
            continue;
        }
        let file = open.into_iter().next()?;
        // Trust the path over the process: a wrapper named `claude` that has a codex transcript
        // open is not a thing I want to record as claude.
        let agent = agent_for_transcript(&file).unwrap_or(row.agent);
        let id = file.file_stem()?.to_string_lossy().into_owned();
        if !valid_id(&id) {
            continue;
        }
        return Some(Session {
            agent: agent.to_string(),
            id,
            file,
        });
    }
    None
}

fn session_path(project_dir: &Path, node_id: &str) -> PathBuf {
    // The node id is a uuid we minted, but it lands in a filename, so it is slugged rather than
    // trusted: this is not the place to discover that an id contained a slash.
    let safe: String = node_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    project_dir
        .join(".identra")
        .join("sessions")
        .join(format!("{safe}.json"))
}

/// Remember which conversation a node was having, so reopening it can pick that one back up.
pub fn save(project_dir: &Path, node_id: &str, session: &Session) -> std::io::Result<()> {
    let path = session_path(project_dir, node_id);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_vec_pretty(session)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, &path)
}

/// The session a node was last having, if its transcript is still there.
///
/// A missing transcript means the agent or the user deleted it, and resuming it would fail at
/// launch for a reason the user cannot see. Better to start fresh and say nothing.
pub fn load(project_dir: &Path, node_id: &str) -> Option<Session> {
    let text = std::fs::read_to_string(session_path(project_dir, node_id)).ok()?;
    let session: Session = serde_json::from_str(&text).ok()?;
    session.file.is_file().then_some(session)
}

/// Forget a node's session, for a node being restarted deliberately fresh.
pub fn forget(project_dir: &Path, node_id: &str) {
    let _ = std::fs::remove_file(session_path(project_dir, node_id));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_only_becomes_an_argument_if_it_looks_like_one() {
        let ok = Session {
            agent: "claude".into(),
            id: "0199a1b2-c3d4-7e8f-9012-3456789abcde".into(),
            file: "/tmp/x.jsonl".into(),
        };
        assert_eq!(
            resume_args(&ok),
            Some(vec![
                "--resume".to_string(),
                "0199a1b2-c3d4-7e8f-9012-3456789abcde".to_string()
            ])
        );
        assert_eq!(
            resume_args(&Session {
                agent: "codex".into(),
                ..ok.clone()
            }),
            Some(vec![
                "resume".to_string(),
                "0199a1b2-c3d4-7e8f-9012-3456789abcde".to_string()
            ])
        );

        // The id comes off a filename, which is not mine, and it goes onto a command line. Anything
        // that could become a second argument or a flag is refused rather than escaped.
        for bad in [
            "",
            "--dangerously-skip",
            "a b",
            "../../etc",
            "a;rm -rf /",
            "a/b",
        ] {
            assert_eq!(
                resume_args(&Session {
                    id: bad.into(),
                    ..ok.clone()
                }),
                None,
                "{bad} must never reach a command line"
            );
        }

        // An agent whose resume flag I do not know gets no invented one.
        assert_eq!(
            resume_args(&Session {
                agent: "gemini".into(),
                ..ok.clone()
            }),
            None
        );
    }

    #[test]
    fn a_transcript_says_which_agent_it_belongs_to() {
        assert_eq!(
            agent_for_transcript(Path::new(
                "/home/me/.claude/projects/-home-me-app/abc.jsonl"
            )),
            Some("claude")
        );
        assert_eq!(
            agent_for_transcript(Path::new("/home/me/.codex/sessions/2026/07/17/xyz.jsonl")),
            Some("codex")
        );
        // Somewhere else entirely is not a session, however much it looks like one.
        assert_eq!(
            agent_for_transcript(Path::new("/tmp/notes/abc.jsonl")),
            None
        );
    }

    #[test]
    fn a_session_round_trips_and_a_deleted_transcript_is_not_resumed() {
        let dir = std::env::temp_dir().join(format!("identra-sess-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let transcript = dir.join("live.jsonl");
        std::fs::write(&transcript, "{}").unwrap();
        let session = Session {
            agent: "claude".into(),
            id: "abc-123".into(),
            file: transcript.clone(),
        };

        assert_eq!(load(&dir, "node-1"), None, "nothing remembered yet");
        save(&dir, "node-1", &session).unwrap();
        assert_eq!(load(&dir, "node-1"), Some(session.clone()));
        // Nodes do not read each other's sessions.
        assert_eq!(load(&dir, "node-2"), None);

        // The user or the agent deleted the transcript. Resuming it would fail at launch for a
        // reason nobody can see, so it is not offered.
        std::fs::remove_file(&transcript).unwrap();
        assert_eq!(load(&dir, "node-1"), None);

        std::fs::write(&transcript, "{}").unwrap();
        assert!(load(&dir, "node-1").is_some());
        forget(&dir, "node-1");
        assert_eq!(
            load(&dir, "node-1"),
            None,
            "a deliberate restart starts fresh"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn detection_reads_this_very_process() {
        // I cannot fake a claude under a pty here, so I check the parts that touch the OS against
        // the one process I definitely have: this one. If /proc reading breaks, this catches it.
        let me = std::process::id();
        assert!(argv0(me).is_some(), "should read my own command line");
        assert!(descendants(me).contains(&me), "the walk includes its root");
        // The test runner has no agent transcript open, so there is no session to find, and the
        // honest answer is nothing rather than a guess.
        assert_eq!(detect(me), None);
    }
}

//! The context bus between agent nodes.
//!
//! The wire on the canvas is the authorization. Two nodes share nothing unless an [`Edge`]
//! joins them: no edge means no peer listing, no context read, no message. Every tool here
//! re-checks the current edges per call, so a wire pulled after launch stops the flow.
//!
//! The peer tools are plain functions over two seams: the edge set (`&[Edge]`, read from
//! `.identra/canvas.json` by the caller) and [`NodeIo`] (the PTY side). That keeps them
//! testable against a fake, no live terminal or HTTP transport needed. `TerminalManager`
//! satisfies `NodeIo`, so wiring the real bus is one blanket impl, not a rewrite.
//!
//! [`tasks`] and [`inbox`] are the other half of working together. Talking coordinates, a board
//! commits, and a queue is what makes talking reliable. All three are separate because they fail
//! differently: a claim has to be atomic, a message has to be delivered exactly once, and a peer
//! read has to reflect the wire that exists right now.

pub mod config;
pub mod inbox;
pub mod server;
pub mod tasks;

use std::path::{Path, PathBuf};

use identra_core::canvas::Edge;
use identra_core::text::{strip_ansi, tail};
use identra_core::TerminalManager;

/// One database per workspace for everything the bus remembers between calls: the task board and
/// the message queue. Both are workspace state with the same lifetime, and one file means one thing
/// to create, back up, or delete with the workspace.
pub fn bus_db_path(project_dir: &Path) -> PathBuf {
    project_dir.join(".identra").join("bus.db")
}

/// Open the workspace's bus database, creating `.identra/` on first use.
///
/// WAL, for the same reason the fact store next to it runs in WAL, and set here because this is
/// the one gate both the board and the inbox open through. Nothing fails without it: rusqlite
/// already defaults the busy timeout to five seconds, so a second writer waits rather than
/// erroring. What the default rollback journal costs is concurrency, and the board is where that
/// shows: the work panel reads it every two seconds while agents claim and complete on it, and
/// under a rollback journal those take turns. WAL lets the poll read while a claim writes.
///
/// The timeout is pinned rather than inherited so the guarantee belongs to this function instead
/// of to a dependency's default, which can change under us without the build saying anything.
pub fn open_bus_db(project_dir: &Path) -> Result<rusqlite::Connection, String> {
    let path = bus_db_path(project_dir);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let conn = rusqlite::Connection::open(path).map_err(|e| e.to_string())?;
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000;")
        .map_err(|e| e.to_string())?;
    Ok(conn)
}

/// Peer transcript tail is capped here. Enough to hand over what a peer just did without
/// shipping a whole scrollback; the tail matters, the head is stale.
const MAX_CONTEXT_BYTES: usize = 8 * 1024;

/// The terminal side the bus reads: a node's transcript. A trait, not `TerminalManager` directly,
/// so the tools test against a fake with no PTY.
pub trait NodeIo {
    /// Current transcript bytes for `id`, or `None` if no such live node.
    fn node_snapshot(&self, id: &str) -> Option<Vec<u8>>;
}

impl NodeIo for TerminalManager {
    fn node_snapshot(&self, id: &str) -> Option<Vec<u8>> {
        // Drop the seq: the bus wants the bytes, not the reattach cursor.
        self.snapshot(id).map(|(bytes, _seq)| bytes)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum BusError {
    /// Caller and peer are not wired. The refusal that makes the edge the authorization.
    NoEdge,
    /// Peer is wired but not running (no snapshot to read).
    NoPeer,
}

/// Node ids that share an edge with `caller`, caller itself excluded, no duplicates.
///
/// Derived from the canvas edges alone. A wired-but-not-yet-launched peer still lists, and that
/// matches the demo flow (wire first, launch after) and any dead peer just yields empty
/// context downstream.
// Edges only, by design: intersecting with a live-id set (NodeIo) would narrow this to
// "running peers", which is not wanted while the demo wires nodes before launching them.
pub fn list_peers(caller: &str, edges: &[Edge]) -> Vec<String> {
    let mut peers = Vec::new();
    for e in edges {
        let other = if e.source == caller {
            &e.target
        } else if e.target == caller {
            &e.source
        } else {
            continue;
        };
        if other != caller && !peers.iter().any(|p| p == other) {
            peers.push(other.clone());
        }
    }
    peers
}

/// Peer's transcript tail as text: ANSI stripped, tail capped at 8 KB. Refuses without an edge.
pub fn get_peer_context<T: NodeIo>(
    caller: &str,
    peer: &str,
    edges: &[Edge],
    io: &T,
) -> Result<String, BusError> {
    if !edged(caller, peer, edges) {
        return Err(BusError::NoEdge);
    }
    let bytes = io.node_snapshot(peer).ok_or(BusError::NoPeer)?;
    Ok(tail(&strip_ansi(&bytes), MAX_CONTEXT_BYTES))
}

/// An edge joins `a` and `b` in either direction.
fn edged(a: &str, b: &str, edges: &[Edge]) -> bool {
    edges
        .iter()
        .any(|e| (e.source == a && e.target == b) || (e.source == b && e.target == a))
}

#[cfg(test)]
mod tests {
    use super::*;
    use identra_core::canvas::Grantor;
    use std::collections::HashMap;

    /// A canvas' worth of terminals with no PTY: fixed snapshots, nothing running.
    struct FakeIo {
        snapshots: HashMap<String, Vec<u8>>,
    }

    impl FakeIo {
        fn new() -> Self {
            FakeIo {
                snapshots: HashMap::new(),
            }
        }
        fn with_snapshot(mut self, id: &str, bytes: &[u8]) -> Self {
            self.snapshots.insert(id.into(), bytes.to_vec());
            self
        }
    }

    impl NodeIo for FakeIo {
        fn node_snapshot(&self, id: &str) -> Option<Vec<u8>> {
            self.snapshots.get(id).cloned()
        }
    }

    fn edge(source: &str, target: &str) -> Edge {
        Edge {
            id: format!("{source}-{target}"),
            source: source.into(),
            target: target.into(),
            by: Grantor::You,
        }
    }

    #[test]
    fn edge_is_the_authorization() {
        let wired = [edge("a", "b")];
        let io = FakeIo::new()
            .with_snapshot("a", b"a's transcript")
            .with_snapshot("b", b"b's transcript");

        // list_peers: the peer appears only when an edge joins them.
        assert_eq!(list_peers("a", &wired), vec!["b".to_string()]);
        assert_eq!(list_peers("a", &[]), Vec::<String>::new());
        // Both directions of the same wire, and no self-listing.
        assert_eq!(list_peers("b", &wired), vec!["a".to_string()]);

        // get_peer_context: refused with no edge, delivered with one.
        assert_eq!(get_peer_context("a", "b", &[], &io), Err(BusError::NoEdge));
        assert_eq!(
            get_peer_context("a", "b", &wired, &io).unwrap(),
            "b's transcript"
        );
    }

    #[test]
    fn context_strips_ansi_and_keeps_the_tail() {
        let wired = [edge("a", "b")];

        // ANSI color codes and a CR are stripped; text survives.
        let ansi = b"\x1b[31mred\x1b[0m\rline\n";
        let io = FakeIo::new().with_snapshot("b", ansi);
        assert_eq!(
            get_peer_context("a", "b", &wired, &io).unwrap(),
            "redline\n"
        );

        // Over-cap snapshot: keep the tail, drop the head.
        let big = format!("HEADMARKER{}TAILMARKER", "x".repeat(9000));
        let io = FakeIo::new().with_snapshot("b", big.as_bytes());
        let ctx = get_peer_context("a", "b", &wired, &io).unwrap();
        assert!(ctx.len() <= MAX_CONTEXT_BYTES);
        assert!(ctx.ends_with("TAILMARKER"));
        assert!(!ctx.contains("HEADMARKER"));
    }

    #[test]
    fn context_refuses_a_wired_but_dead_peer() {
        let wired = [edge("a", "b")];
        let io = FakeIo::new(); // no snapshot for b
        assert_eq!(
            get_peer_context("a", "b", &wired, &io),
            Err(BusError::NoPeer)
        );
    }

    /// The board and the inbox both open through `open_bus_db`, so the pragmas are pinned once
    /// here rather than in each of them. The busy timeout is asserted even though rusqlite already
    /// defaults to it: the point of setting it is that this function owns the value, and a test
    /// that only checked WAL would not notice the day that default moved.
    #[test]
    fn the_bus_db_opens_in_wal_with_a_busy_timeout() {
        let dir = std::env::temp_dir().join(format!("identra-busdb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let conn = open_bus_db(&dir).unwrap();

        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal");
        let timeout: i64 = conn
            .query_row("PRAGMA busy_timeout", [], |r| r.get(0))
            .unwrap();
        assert!(timeout >= 5000, "the busy timeout is set, got {timeout}");

        // Opening it created the directory it lives in, which is the other half of this function.
        assert!(bus_db_path(&dir).exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }
}

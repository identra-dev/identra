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
//! [`tasks`] is the other half of working together: talking coordinates, a board commits. It is
//! separate because a claim has to be atomic, which is a database's job, not a message's.

pub mod config;
pub mod server;
pub mod tasks;

use identra_core::canvas::Edge;
use identra_core::TerminalManager;

/// Peer transcript tail is capped here. Enough to hand over what a peer just did without
/// shipping a whole scrollback; the tail matters, the head is stale.
const MAX_CONTEXT_BYTES: usize = 8 * 1024;

/// The terminal side the bus touches: read a node's transcript, push bytes to its stdin.
/// A trait, not `TerminalManager` directly, so the tools test against a fake with no PTY.
pub trait NodeIo {
    /// Current transcript bytes for `id`, or `None` if no such live node.
    fn node_snapshot(&self, id: &str) -> Option<Vec<u8>>;
    /// Write bytes to `id`'s stdin.
    fn node_input(&self, id: &str, data: &[u8]) -> Result<(), String>;
}

impl NodeIo for TerminalManager {
    fn node_snapshot(&self, id: &str) -> Option<Vec<u8>> {
        // Drop the seq: the bus wants the bytes, not the reattach cursor.
        self.snapshot(id).map(|(bytes, _seq)| bytes)
    }
    fn node_input(&self, id: &str, data: &[u8]) -> Result<(), String> {
        self.input(id, data).map_err(|e| e.to_string())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum BusError {
    /// Caller and peer are not wired. The refusal that makes the edge the authorization.
    NoEdge,
    /// Peer is wired but not running (no snapshot to read).
    NoPeer,
    /// The underlying PTY write failed.
    Input(String),
}

/// Node ids that share an edge with `caller`, caller itself excluded, no duplicates.
///
/// Derived from the canvas edges alone. A wired-but-not-yet-launched peer still lists — that
/// matches the demo flow (wire first, launch after) and any dead peer just yields empty
/// context downstream.
// Edges only, by design: intersecting with a live-id set (NodeIo) would narrow this to
// "running peers" — not wanted while the demo wires nodes before launching them.
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

/// Inject `[from <caller>] text\n` into the peer's stdin. Refuses without an edge.
///
/// `caller_title` is the peer-facing label (a node's title); falls back to the id when blank.
pub fn send_to_node<T: NodeIo>(
    caller: &str,
    caller_title: &str,
    peer: &str,
    text: &str,
    edges: &[Edge],
    io: &T,
) -> Result<(), BusError> {
    if !edged(caller, peer, edges) {
        return Err(BusError::NoEdge);
    }
    let label = if caller_title.is_empty() {
        caller
    } else {
        caller_title
    };
    let line = format!("[from {label}] {text}\n");
    io.node_input(peer, line.as_bytes())
        .map_err(BusError::Input)
}

/// An edge joins `a` and `b` in either direction.
fn edged(a: &str, b: &str, edges: &[Edge]) -> bool {
    edges
        .iter()
        .any(|e| (e.source == a && e.target == b) || (e.source == b && e.target == a))
}

/// Strip terminal escape noise, keep the readable text. Drops CSI (`ESC [ … final`) and
/// OSC (`ESC ] … BEL|ST`) sequences, other two-byte escapes, and bare control bytes except
/// `\n`/`\t`. Output is valid UTF-8 (lossy), so a later tail slice can't split an escape.
fn strip_ansi(bytes: &[u8]) -> String {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            0x1b => {
                i += 1;
                match bytes.get(i) {
                    Some(b'[') => {
                        // CSI: params/intermediates until a final byte in 0x40..=0x7e.
                        i += 1;
                        while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                            i += 1;
                        }
                        i += 1;
                    }
                    Some(b']') => {
                        // OSC: runs until BEL or the ST terminator (ESC \).
                        i += 1;
                        while i < bytes.len() {
                            if bytes[i] == 0x07 {
                                i += 1;
                                break;
                            }
                            if bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'\\') {
                                i += 2;
                                break;
                            }
                            i += 1;
                        }
                    }
                    Some(_) => i += 1, // other ESC x: drop the pair
                    None => {}
                }
            }
            b'\n' | b'\t' => {
                out.push(b);
                i += 1;
            }
            _ if b < 0x20 || b == 0x7f => i += 1, // CR, BEL, BS, DEL, …
            _ => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Last `max` bytes of `s`, snapped forward to a char boundary so the slice stays valid UTF-8.
fn tail(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut start = s.len() - max;
    while start < s.len() && !s.is_char_boundary(start) {
        start += 1;
    }
    s[start..].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// A canvas' worth of terminals with no PTY: fixed snapshots, recorded inputs.
    struct FakeIo {
        snapshots: HashMap<String, Vec<u8>>,
        inputs: RefCell<Vec<(String, Vec<u8>)>>,
    }

    impl FakeIo {
        fn new() -> Self {
            FakeIo {
                snapshots: HashMap::new(),
                inputs: RefCell::new(Vec::new()),
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
        fn node_input(&self, id: &str, data: &[u8]) -> Result<(), String> {
            self.inputs.borrow_mut().push((id.into(), data.to_vec()));
            Ok(())
        }
    }

    fn edge(source: &str, target: &str) -> Edge {
        Edge {
            id: format!("{source}-{target}"),
            source: source.into(),
            target: target.into(),
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

        // send_to_node: no edge => nothing reaches the peer.
        assert_eq!(
            send_to_node("a", "Node A", "b", "hi", &[], &io),
            Err(BusError::NoEdge)
        );
        assert!(io.inputs.borrow().is_empty());

        // With the edge, the message lands on b's stdin with the from-prefix.
        send_to_node("a", "Node A", "b", "build the route", &wired, &io).unwrap();
        let recorded = io.inputs.borrow();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].0, "b");
        assert_eq!(recorded[0].1, b"[from Node A] build the route\n");
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

    #[test]
    fn send_falls_back_to_id_when_title_blank() {
        let wired = [edge("a", "b")];
        let io = FakeIo::new();
        send_to_node("a", "", "b", "yo", &wired, &io).unwrap();
        assert_eq!(io.inputs.borrow()[0].1, b"[from a] yo\n");
    }
}

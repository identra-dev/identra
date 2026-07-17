//! The bus over real HTTP, spoken the way an agent CLI speaks it.
//!
//! The unit tests cover the dispatch and the edge gate directly. What they cannot show is that the
//! whole path works: a socket, the identity header a CLI expands from its env, the JSON-RPC
//! envelope, and a tool result coming back. That path is the entire claim ("two agents can share
//! context"), so I drive it here with a hand-written request rather than trusting the layers to
//! line up. Writing the request by hand is also what lets me forge one: a real client would never
//! send someone else's id, which is exactly the case worth testing.
//!
//! I write the request by hand instead of pulling in an HTTP client for one test. `Connection:
//! close` makes the server hang up when it is done, so reading to EOF is the whole response and I
//! never have to parse keep-alive framing.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use identra_core::canvas::{self, Canvas, Edge, Node};
use identra_core::TerminalManager;
use identra_mcp::server::{bind, serve, Bus};

fn post(port: u16, token: &str, body: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("bus is listening");
    let req = format!(
        "POST /mcp HTTP/1.1\r\n\
         Host: 127.0.0.1\r\n\
         Content-Type: application/json\r\n\
         X-Identra-Token: {token}\r\n\
         Content-Length: {len}\r\n\
         Connection: close\r\n\r\n{body}",
        len = body.len()
    );
    stream.write_all(req.as_bytes()).expect("write request");
    let mut out = String::new();
    stream.read_to_string(&mut out).expect("read response");
    out
}

fn node(id: &str, title: &str) -> Node {
    Node {
        id: id.into(),
        kind: "codex".into(),
        x: 0.0,
        y: 0.0,
        width: 480.0,
        height: 320.0,
        title: title.into(),
        cwd: None,
    }
}

#[test]
fn an_agent_reaches_the_bus_and_sees_only_the_peer_it_is_wired_to() {
    let dir = std::env::temp_dir().join(format!("identra-bus-http-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    // A canvas with three nodes where only a and b are wired. c is the control: a valid caller
    // still must not see it, because the edge is the authorization.
    canvas::save(
        &dir,
        &Canvas {
            nodes: vec![node("a", "Route"), node("b", "Tests"), node("c", "Docs")],
            edges: vec![Edge {
                id: "e1".into(),
                source: "a".into(),
                target: "b".into(),
            }],
            ..Canvas::default()
        },
    )
    .expect("seed the canvas");

    let manager = Arc::new(TerminalManager::new(Arc::new(|_id, _out| {})));
    // No canvas in this test: nothing here asks the window to change anything.
    let bus = Arc::new(Bus::new(
        manager,
        Arc::new(Mutex::new(dir.clone())),
        Arc::new(|_cmd| {}),
    ));
    // Each node is launched with its own secret. That is the only thing that says who it is.
    let token = bus.issue_token("a");
    let token_c = bus.issue_token("c");
    let (listener, port) = bind().expect("bind the bus");

    let serving = bus.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let _ = rt.block_on(serve(listener, serving));
    });

    // The listener is already bound, so the port answers as soon as the task picks it up.
    let deadline = Instant::now() + Duration::from_secs(5);
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < deadline, "bus never came up");
        std::thread::sleep(Duration::from_millis(20));
    }

    // The handshake a CLI opens with. I echo its protocol version so it agrees with whatever the
    // agent negotiates rather than pinning one both sides have to share.
    let init = post(
        port,
        &token,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#,
    );
    assert!(init.starts_with("HTTP/1.1 200"), "init failed: {init}");
    assert!(init.contains("\"protocolVersion\":\"2025-03-26\""));
    assert!(init.contains("identra-bus"));

    // The tools an agent is told it has.
    let tools = post(
        port,
        &token,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
    );
    for tool in ["list_peers", "get_peer_context", "send_to_node"] {
        assert!(tools.contains(tool), "{tool} missing from tools/list");
    }

    // The payoff: a sees b, by name, because a wire joins them. And a does not see c.
    let peers = post(
        port,
        &token,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_peers"}}"#,
    );
    assert!(peers.contains("Tests"), "a should see its peer b: {peers}");
    assert!(
        !peers.contains("Docs"),
        "an unwired node must stay invisible"
    );

    // c's own secret names c, which has no edges: a valid agent, no peers. c cannot borrow a's view
    // by asking nicely, because the request carries no node id to ask with.
    let alone = post(
        port,
        &token_c,
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"list_peers"}}"#,
    );
    assert!(alone.contains("no wired peers"));

    // Impersonation: c presents its real secret but names itself "a" every way the wire allows, in
    // a header and in the tool arguments. Neither is read, so c stays c and never sees a's peer.
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let body = r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"list_peers","arguments":{"nodeId":"a","caller":"a"}}}"#;
    let req = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
         X-Identra-Token: {token_c}\r\nX-Identra-Node: a\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(req.as_bytes()).expect("write");
    let mut spoofed = String::new();
    stream.read_to_string(&mut spoofed).expect("read");
    assert!(
        spoofed.contains("no wired peers"),
        "a forged node id must not grant a's view: {spoofed}"
    );
    assert!(!spoofed.contains("Tests"), "impersonation leaked a's peer");

    // A process that found the port but holds no secret at all gets nothing.
    let forged = post(
        port,
        "not-the-token",
        r#"{"jsonrpc":"2.0","id":6,"method":"tools/list"}"#,
    );
    assert!(forged.starts_with("HTTP/1.1 401"), "expected 401: {forged}");

    std::fs::remove_dir_all(&dir).expect("clean up");
}

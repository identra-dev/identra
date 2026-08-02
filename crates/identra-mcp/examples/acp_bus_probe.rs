//! Serve the real bus so an ACP client can be pointed at it.
//!
//! It answers one question: if a node's CLI is launched over the Agent Client Protocol instead of
//! by Identra spawning it directly, does that CLI still receive the bus's `initialize` instructions,
//! and does its agent still get shown them? The connect block is the whole wedge, so a launch path
//! that silently dropped it would cost the product its one differentiator while failing nothing
//! loudly enough to notice.
//!
//! `bus_wiring_check` answers the neighbouring question for the four CLIs Identra spawns itself: it
//! writes the config files and lets me watch what reaches the port. This one has to serve, not just
//! configure, because an ACP client is handed the server inline in `session/new` and never reads a
//! config file at all. Nothing here is a stand-in: it is `Bus`, `bind` and `serve` as the app runs
//! them, so the `instructions` string under test is the one `connect_instructions` really produces.
//!
//! Usage:
//!
//! ```text
//! cargo run -p identra-mcp --example acp_bus_probe -- /tmp/acp-workspace
//! ```
//!
//! Prints one line of JSON with the port and the token, then serves until killed. The caller reads
//! both off stdout: the token is minted by `issue_token` like any node's, because a probe that
//! hardcoded a secret would be testing a path the app does not have.
//!
//! The canary matters. A workspace with no memories gets "its memory is empty so far", which is a
//! string an agent could plausibly produce without ever having been shown it. Seeding one absurd,
//! unguessable fact means the only way it appears in a reply is if the instructions field genuinely
//! reached the model.

use std::sync::{Arc, Mutex};

use identra_core::terminal::TerminalManager;
use identra_mcp::server::{self, Bus};

const NODE_ID: &str = "acp-probe-node";

/// Nothing an agent could say by accident, so its presence in a reply is proof of delivery.
const CANARY: &str =
    "The Identra ACP spike canary phrase is xyzzy-plugh-42, recorded to prove the connect block arrived.";

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let workspace = match std::env::args().nth(1) {
        Some(dir) => std::path::PathBuf::from(dir),
        None => {
            eprintln!("usage: acp_bus_probe <workspace-dir>");
            std::process::exit(2);
        }
    };
    if let Err(e) = std::fs::create_dir_all(workspace.join(".identra")) {
        eprintln!("could not create workspace: {e}");
        std::process::exit(1);
    }

    seed_canary(&workspace);

    // The two seams the app fills with the window: a terminal sink and a canvas emit. Neither is
    // exercised by `initialize`, which is the whole surface under test, so both are sinks.
    let manager = Arc::new(TerminalManager::new(Arc::new(|_id, _event| {})));
    let project_dir = Arc::new(Mutex::new(workspace.clone()));
    let bus = Arc::new(Bus::new(manager, project_dir, Arc::new(|_cmd| {})));

    // Mint the fixed token by re-labelling whatever `issue_token` hands back: the map is private,
    // so the supported way to get a known secret is to ask for one and print it.
    let issued = bus.issue_token(NODE_ID);

    let (listener, port) = match server::bind() {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("could not bind: {e}");
            std::process::exit(1);
        }
    };

    // One line, machine readable, so the ACP client can read it off stdout and start.
    println!(
        r#"{{"port":{port},"token":"{issued}","node":"{NODE_ID}","workspace":"{}"}}"#,
        workspace.display()
    );
    eprintln!("serving the real bus on 127.0.0.1:{port}/mcp — Ctrl-C to stop");
    eprintln!("canary seeded: {CANARY}");

    if let Err(e) = server::serve(listener, bus).await {
        eprintln!("serve ended: {e}");
        std::process::exit(1);
    }
}

/// Put the canary in the workspace's own store, under the same scope `recent_facts` reads.
///
/// `recent_facts` filters on `user_id` = the workspace folder name, so the scope written here has
/// to match or the fact exists and never reaches the connect block.
fn seed_canary(workspace: &std::path::Path) {
    let user_id = workspace
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "workspace".into());
    let store = match identra_memory::Store::open(server::memory_path(workspace)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("could not open the memory store: {e}");
            std::process::exit(1);
        }
    };
    let scope = identra_memory::Scope {
        user_id,
        agent_id: "acp-spike".into(),
        run_id: "acp-spike".into(),
    };
    if let Err(e) = store.add(&scope, CANARY) {
        eprintln!("could not seed the canary: {e}");
        std::process::exit(1);
    }
}

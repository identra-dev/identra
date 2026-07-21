//! Generate the bus config for each agent into a directory, so the real CLIs can be pointed at it.
//!
//! The unit tests pin the JSON shape, but the shape is only right if the CLI actually agrees with
//! it, and no unit test can tell me that: the interpolation syntax, the key names, and the folder
//! trust rule are all facts that live inside somebody else's binary. So this writes the same files
//! `activate()` writes, and then I drive the real `gemini` and `opencode` against them and watch
//! what lands on the bus.
//!
//! Usage:
//!
//! ```text
//! cargo run -p identra-mcp --example bus_wiring_check -- /tmp/some-workspace 8900
//! cd /tmp/some-workspace && IDENTRA_BUS_TOKEN=secret gemini mcp list
//! ```
//!
//! The check is that the request reaching port 8900 carries `X-Identra-Token: secret`. If it
//! arrives empty or with the literal `${IDENTRA_BUS_TOKEN}`, that CLI's interpolation is spelled
//! differently from what `config.rs` assumes and the node would join the bus as nobody.

use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let workspace = match args.next() {
        Some(dir) => PathBuf::from(dir),
        None => {
            eprintln!("usage: bus_wiring_check <workspace-dir> [port]");
            std::process::exit(2);
        }
    };
    let port: u16 = args.next().and_then(|p| p.parse().ok()).unwrap_or(8900);

    if let Err(e) = identra_mcp::config::write_mcp_json(&workspace, port) {
        eprintln!("could not write .mcp.json: {e}");
        std::process::exit(1);
    }
    if let Err(e) = identra_mcp::config::write_gemini_settings(&workspace, port) {
        eprintln!("could not write .gemini/settings.json: {e}");
        std::process::exit(1);
    }
    if let Err(e) = identra_mcp::config::write_opencode_config(&workspace, port) {
        eprintln!("could not write .identra/opencode.json: {e}");
        std::process::exit(1);
    }

    println!(
        "wrote bus config for port {port} into {}",
        workspace.display()
    );
    for kind in ["codex", "claude", "gemini", "opencode"] {
        let launch = identra_mcp::config::launch_args(kind, port, &workspace);
        let env = identra_mcp::config::launch_env(kind, port, "secret", "node-a", &workspace);
        let extra: Vec<String> = env
            .iter()
            .filter(|(k, _)| k == "OPENCODE_CONFIG")
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        println!("  {kind}: args {launch:?} {}", extra.join(" "));
    }
}

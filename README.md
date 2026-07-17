<div align="center">

# Identra

**A desktop canvas for running coding agents.**

You drop an agent onto the board, it runs in a real terminal inside a node, you wire nodes
together, and Identra keeps a memory of what happened so the next agent you open already
knows the project.

[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](./LICENSE)
[![Platform](https://img.shields.io/badge/platform-Linux-E95420.svg)](#requirements)
[![Stack](https://img.shields.io/badge/stack-Rust%20%2B%20Tauri%20%2B%20React-24C8DB.svg)](#how-it-works)
[![Status](https://img.shields.io/badge/status-early-orange.svg)](#status)

[Why](#why-i-built-this) &nbsp;·&nbsp;
[What it does](#what-it-does) &nbsp;·&nbsp;
[Requirements](#requirements) &nbsp;·&nbsp;
[Quick start](#quick-start) &nbsp;·&nbsp;
[How it works](#how-it-works) &nbsp;·&nbsp;
[Tasks](#tasks) &nbsp;·&nbsp;
[Status](#status)

</div>

Rust engine, Tauri shell, React canvas. Apache-2.0.

---

## Why i built this

Every coding agent I run forgets everything the moment I close it. New session, new agent,
new teammate, and I am back to explaining the same things: how the code is laid out, which
approach we already tried and threw away, the constraints that are not written down anywhere.
The agent is sharp in the moment and blank the next morning.

So the context ends up scattered across a dozen terminal tabs and my own head, and none of it
survives. Identra is my fix for that. The canvas is where the work happens. The memory layer
sits underneath it and quietly keeps the parts worth keeping, then hands them back to the next
agent so nobody starts from zero.

## What it does

- Drop an agent node on an infinite canvas. It spawns a real `codex` process in a terminal
  you can type into. No shell wrapper, no fake sandbox, the actual CLI on your machine.
- The canvas saves itself. Close the app, open it next week, your nodes and layout come back
  exactly where you left them.
- A memory layer watches your sessions and pulls out the durable facts: decisions, rejected
  directions, conventions. When you open a fresh agent in a project Identra already knows, it
  shows what it remembers before the agent's first prompt.
- Everything runs on your machine. Your agent API keys stay in your own CLI config. Identra
  never stores them and never phones home.

The look is Ubuntu/Yaru, because I stare at this thing all day and I wanted it to feel like
part of my desktop, not a browser tab pretending to be an app.

## Requirements

Identra is a Linux desktop app (it renders through webkitgtk). You need:

- Rust and Cargo (`rustup` is the easy way)
- The Tauri CLI: `cargo install tauri-cli`
- [bun](https://bun.sh) for the web side
- [just](https://github.com/casey/just) for the tasks below
- webkitgtk and its build deps. On Debian/Ubuntu:

  ```bash
  sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
    libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
  ```

- `codex` on your PATH if you want an agent node to actually run something. Without it, a node
  still opens, it just tells you the binary is missing instead of pretending to work.

Run `just doctor` and it will tell you what you are missing.

## Quick start

```bash
git clone https://github.com/identra-dev/identra.git
cd identra
just doctor      # check your machine is ready
just dev         # build and launch with hot reload
```

First launch builds the Rust side, so give it a minute. After that, right-click the canvas or
use the dock to add a Codex node, and start typing.

## How it works

Identra is a small Rust engine with thin shells on top of it.

```
identra/
  crates/
    identra-core     the engine: PTY/terminal manager, canvas store, agent detection
    identra-memory   the memory layer: fact extraction, local embeddings, history
    identra-mcp      MCP servers that expose memory (and, later, an agent bus)
  apps/
    identra-desktop  Tauri v2 + React: the canvas, the nodes, the dock
  presets/
    identra-agents   agent presets and orchestration recipes
  docs/             architecture and design notes
```

The engine owns the hard parts. A node in the UI is a thin client that talks to `identra-core`
over a small typed command channel (`terminal:start`, `terminal:input`, and so on). Output
streams back with a sequence number so a node can reattach after a reload without dropping or
duplicating a line. The canvas is the source of truth for layout, and it saves to
`.identra/canvas.json` in your project with a debounced atomic write, so a fast drag never
thrashes your disk.

Memory is its own crate. After a session it runs one extraction pass, dedupes by content hash,
embeds locally with fastembed, and stores the result in a single SQLite file with a vector
index. If you have no model configured, it stores the raw text instead of guessing. Memory
degrades quietly, it never blocks your work and it never calls out to a server you did not ask
it to.

## Where your data lives

- Canvas state: `.identra/canvas.json` inside each project (gitignored by default).
- Memory: a local SQLite database. Nothing leaves your machine.
- Secrets: none of Identra's business. Your agent keys stay in your agent's own config.

## Tasks

Identra uses a `justfile` for everything. Run `just` to list them.

| Task | What it does |
|------|--------------|
| `just dev` | Build and run with hot reload |
| `just build` | Produce a release bundle (AppImage and .deb on Linux) |
| `just test` | Run the Rust and web test suites |
| `just fmt` | Format Rust and web code |
| `just lint` | Clippy and the web linter, warnings fail |
| `just check` | Format, lint, test, and the trace gate. This is what I run before a commit |
| `just doctor` | Check your machine has the tools to build and run |

## Status

Early, and honest about it. The first milestone is a single agent node running a real `codex`
in a persistent canvas, with the memory layer recalling context on a fresh node. Multi-agent
hand-off between wired nodes is designed but not the focus yet. If something is rough, it is
because I would rather ship the core working than a wide surface half working.

## Contributing

Issues and pull requests are welcome. Two house rules: run `just check` before you push, and
keep the code readable by a human six months from now. Comments explain why, not what.

## License

Apache-2.0. See [LICENSE](./LICENSE) and [NOTICE](./NOTICE).

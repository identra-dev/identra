<div align="center">

# Identra

**A desktop canvas for running coding agents.**

You drop an agent onto the board, it runs in a real terminal inside a node, you wire nodes
together, and Identra keeps a memory of what happened so the next agent you open already
knows the project.

[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](./LICENSE)
[![Platform](https://img.shields.io/badge/platform-Linux%20%7C%20macOS-E95420.svg)](#requirements)
[![Stack](https://img.shields.io/badge/stack-Rust%20%2B%20Tauri%20%2B%20React-24C8DB.svg)](#how-it-works)
[![Status](https://img.shields.io/badge/status-early-orange.svg)](#status)

[Download and run](#download-and-run) &nbsp;·&nbsp;
[Why](#why-i-built-this) &nbsp;·&nbsp;
[What it does](#what-it-does) &nbsp;·&nbsp;
[Requirements](#requirements) &nbsp;·&nbsp;
[Build from source](#build-from-source) &nbsp;·&nbsp;
[How it works](#how-it-works) &nbsp;·&nbsp;
[Status](#status)

</div>

Rust engine, Tauri shell, React canvas. Apache-2.0.

---

## Download and run

Grab a build from [Releases](https://github.com/identra-dev/identra/releases). Nothing to
compile.

**Linux**

```bash
chmod +x Identra_0.1.0_amd64.AppImage
./Identra_0.1.0_amd64.AppImage
```

There is a `.deb` and an `.rpm` there too if you would rather install it properly.

**macOS**

The `.dmg` is built for Apple Silicon. Intel Macs are not supported yet: the local embedding
model's runtime ships no x86 macOS build, and I would rather say that plainly than ship a dmg
that half works.

Open the `.dmg` and drag Identra to Applications. It is not notarized yet, so the first launch
needs one of these, or macOS will tell you it cannot check the app for malicious software:

```bash
xattr -cr /Applications/Identra.app
```

Or right click the app and choose Open, which gives you the same dialog with an Open button on
it.

**One thing to install first.** Identra runs coding agents, it does not include one. Put
[codex](https://github.com/openai/codex) on your PATH and sign in:

```bash
codex --version   # if this works, Identra can run it
```

Without it a node still opens. It tells you the binary is missing rather than pretending to
work, but you will not see an agent run.

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
- Type what you want done into the command bar and an orchestrator seat breaks the work up,
  spawns the helper nodes it needs, wires them, and puts the pieces on a shared task board you
  can watch. Wired agents message each other, split work, and hand results back over a local
  bus. An edge between two nodes is the permission: no wire, no shared context.
- A workspace is a folder, usually your repo. Open one you have, make an empty one, or paste a
  git URL and Identra clones it. Each workspace keeps its own canvas, board, memory, and
  wallpaper, and the picker shows each board at a glance.
- Press Run and Identra starts your project's own dev command in a node, reads the local URL
  off the server's banner, and one click opens the page in a browser node beside it.
- Reading a conversation in a tile is squinting, so every node expands to the full window over
  the same terminal. Drop a file on the canvas to view it, browse and search the workspace from
  the Files panel, and agents can open a file node themselves to show you what they made.
- The canvas saves itself. Close the app, open it next week, your nodes, layout, and each
  agent's own conversation come back exactly where you left them.
- A memory layer watches your sessions and pulls out the durable facts: decisions, rejected
  directions, conventions. When you open a fresh agent in a project Identra already knows, it
  shows what it remembers before the agent's first prompt.
- Everything runs on your machine. Your agent API keys stay in your own CLI config. Identra
  never stores them and never phones home. Installed builds update themselves from Releases,
  and installing an update is always your click, never automatic.

The look is Ubuntu/Yaru, because I stare at this thing all day and I wanted it to feel like
part of my desktop, not a browser tab pretending to be an app. The wallpaper is yours to set,
per workspace.

## Requirements

**To run a release build:** `codex` on your PATH, and nothing else. Linux or macOS.

Windows is not supported yet. It is not a small port: interactive agent TUIs need ConPTY, the
paths differ, and WebView2 wants its own testing. It is on the list, it is not this month.

**To build from source, you also need:**

- Rust and Cargo (`rustup` is the easy way)
- The Tauri CLI: `cargo install tauri-cli`
- [bun](https://bun.sh) for the web side
- [just](https://github.com/casey/just) for the tasks below
- On Linux, webkitgtk and its build deps. On Debian and Ubuntu:

  ```bash
  sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
    libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
  ```

  macOS renders through WKWebView, which is already there, so there is nothing to install.

Run `just doctor` and it will tell you what you are missing.

## Build from source

```bash
git clone https://github.com/identra-dev/identra.git
cd identra
just doctor      # check your machine is ready
just dev         # build and launch with hot reload
```

First launch builds the Rust side, so give it a minute. After that, right-click the canvas or
use the dock to add a Codex node, and start typing.

`just build` produces the release bundles for whatever OS you are on. Run it from
`apps/identra-desktop` if you call `cargo tauri build` yourself: the frontend build script it
shells out to lives there, not at the repo root.

## How it works

Identra is a small Rust engine with thin shells on top of it.

```
identra/
  crates/
    identra-core     the engine: PTY/terminal manager, canvas store, agent detection
    identra-memory   the memory layer: fact extraction, local embeddings, history
    identra-mcp      the context bus: an MCP server wired agents share context through
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

## Built with Codex

<!-- TODO before submitting. Fill this in yourself, from what you actually did. Do not let anyone
     draft it for you and do not soften it: it is a statement to the judges about how this was
     built, and the only version worth writing is the true one. Name the parts Codex wrote or
     unblocked, say which model, and if GPT-5.6 access never landed then say the model you really
     used. A thin honest paragraph beats a thick invented one, and an invented one is the only
     thing here that can actually cost you the entry. Delete this comment when it is written. -->

## Where your data lives

- Canvas state: `.identra/canvas.json` inside each project (gitignored by default).
- Memory: a local SQLite database. Nothing leaves your machine.
- The embedding model: fetched once into your OS cache directory, under `identra/models`. This is
  the only thing Identra downloads, it is the model itself, and your memories are never part of it.
  `IDENTRA_EMBEDDINGS=off` turns it off.
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
| `just check` | Format, lint, and test. This is what I run before a commit |
| `just doctor` | Check your machine has the tools to build and run |

## Status

Early, and honest about it. What works today: agent nodes each running their own real CLI in a
persistent canvas, the command bar driving an orchestrator seat that spawns and wires helpers,
a shared task board, workspaces with clone and per-board wallpaper, the dev server Run button
with a live browser preview, full-window focus on any node, file viewing on the canvas, the
Files panel with search, and auto update. Connection edges let two wired agents share context
through the bus (draw the wire, then launch, since a CLI reads its MCP servers at startup).
Memory is on the recall path: agents read and write it through the bus, and the work panel
shows you every fact the project holds, because memory only agents can read is memory you
cannot check or correct.

Recall works on meaning, not on words. Ask "how do we handle auth" and you get the decision that
was written down as "the API issues JWT bearer tokens", which share no word at all. That runs on a
small model on your machine (about 130MB, fetched once the first time you use memory, then it works
offline). If you would rather it never reach the network, set `IDENTRA_EMBEDDINGS=off` and recall
falls back to matching words, which is worse but yours.

What it will not do is pretend. A search hands back the closest facts it has and says that is what
they are, because the model's ranking is good but its scores cannot tell a real answer from a
question about something this project never touched. Judging that is the agent's job, and it is
better at it than a threshold would be.

The browser node ships as a live preview (see `docs/browser-bridge.md` for why the agent-drive half
needs Chromium).

If something is rough, it is because I would rather ship the core working than a wide surface half
working.

## Contributing

Issues and pull requests are welcome. Two house rules: run `just check` before you push, and
keep the code readable by a human six months from now. Comments explain why, not what.

## License

Apache-2.0. See [LICENSE](./LICENSE) and [NOTICE](./NOTICE).

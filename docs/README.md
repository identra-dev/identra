# Identra documentation

A desktop shell for running coding agents, with a memory they open already holding. You open an
agent in a tab, it runs in a real terminal, you connect the ones that should read each other's
work, and Identra keeps a memory of what happened — which the next agent receives in its opening
handshake, without calling a tool to ask.

New here? The [README](../README.md) is the front door: what it is, how to install it, and what it
does. These pages are the depth behind it.

## Pages

| Page | What it answers |
|------|-----------------|
| [Agents](./agents.md) | Which CLIs Identra runs, what each is allowed to do and why, how connecting works |
| [Memory](./memory.md) | What a project remembers, how recall by meaning works, how to turn it off |
| [Troubleshooting](./troubleshooting.md) | The failures people actually hit, and what each one means |
| [The browser tab](./browser-bridge.md) | The live preview, and the platform limit behind agent-driven browsing |

## Start here

Two things get you running:

```bash
# 1. Install Identra
curl -fsSL https://raw.githubusercontent.com/identra-dev/identra/main/install.sh | sh

# 2. Have a coding agent on your PATH. Identra runs them, it does not ship one.
claude --version
```

Then open a folder as a workspace and open an agent from the sidebar. If step 2 came up empty,
[Agents](./agents.md) covers what to install and why Claude Code is the one to start with;
[Troubleshooting](./troubleshooting.md) covers the case where you have one installed and Identra
cannot see it.

## The short version of how it works

A small Rust engine with thin shells on top.

- **`identra-core`** owns the hard parts: the PTY terminal manager that spawns real agent CLIs,
  the workspace store, agent detection, workspaces, the dev server and file browsing.
- **`identra-memory`** is the memory layer: fact extraction, local embeddings, history.
- **`identra-mcp`** is the context bus, an MCP server that connected agents share context through.
- **`identra-desktop`** is the Tauri and React shell: the three columns, the panes, the memory panel.

A pane in the UI is a thin client talking to the engine over a small typed command channel. Output
streams back with a sequence number, so a pane reattaches after a reload without dropping or
duplicating a line. What a workspace holds — which agents are open, and which are connected —
saves itself to `.identra/canvas.json` in your project.

## Everything runs on your machine

No account, no cloud, no phone-home. Your agent API keys stay in your agent's own config, and
Identra neither stores nor forwards them. Memory is a local SQLite file per workspace. The embedding
model ships inside a release build, so recall works offline on first launch.

The full list of what Identra writes into a folder you open is in the
[README](../README.md#what-identra-writes-into-your-project), because it is your repository and a
promise that it is tidy is worth less than the list.

## Contributing

Issues and pull requests are welcome. Two house rules: run `just check` before you push, and keep
the code readable by a human six months from now. Comments explain why, not what. See
[CONTRIBUTING.md](../CONTRIBUTING.md).

Licensed Apache-2.0.

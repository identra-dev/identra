# Agents

Identra does not include a coding agent. It runs the ones already on your machine, in real
terminals, and gets out of the way. This page is what it launches, what each one is allowed to do,
and why one of them starts out knowing more than the others.

## The four it runs

| Agent | Where to get it | On the bus | Sees project memory on connect |
|-------|-----------------|-----------|-------------------------------|
| Claude Code | [claude.com/claude-code](https://claude.com/claude-code) | yes | **yes** |
| codex | [github.com/openai/codex](https://github.com/openai/codex) | yes | no |
| gemini | its own CLI, on your `PATH` | yes | no |
| opencode | its own CLI, on your `PATH` | yes | no |

Identra points at an install page rather than printing a shell command, on purpose: an install line
drifts by OS, by version and by package manager, and a wrong one is worse than no line at all. For
gemini and opencode, install however that project currently recommends; Identra finds them on your
`PATH` either way.

You need at least one. A node still opens without any of them, but it names the agents you do not
have instead of pretending to run something, and nothing spawns.

Check what Identra can see:

```bash
just doctor
```

Mix them freely in one workspace. Nothing about a workspace is tied to a particular agent.

## Why Claude Code is the one to start with

All four run. Only one of them shows you the headline feature working on the first try.

A project's memory reaches an agent over the MCP handshake, in the `instructions` field the bus
returns at `initialize`. Claude Code surfaces what arrives there; the others accept the connection
and do not show it. So an agent you open in a project Identra already knows starts that session
already briefed if it is Claude Code, and starts blank otherwise.

The others are not second-class for anything else. They spawn, they connect, they share the task
board, they read and write memory through the bus tools. They just start their first session
without the free briefing.

## What an agent may do without asking

This is the one setting worth reading before you leave Identra running on something you care
about. It lives in Settings, and it has two positions.

### Bypass (the default)

Every prompt each CLI has a switch for is turned off: approvals, sandboxing, directory trust, and
consenting to Identra's own context bus. The agent starts working the moment it opens and connects
to the others on its own.

Each CLI spells it differently, and Identra passes the one that CLI understands:

| Agent | Flag |
|-------|------|
| codex | `--dangerously-bypass-approvals-and-sandbox` |
| Claude Code | `--dangerously-skip-permissions` |
| gemini | `--yolo` |
| opencode | `--auto` |

gemini additionally gets `--skip-trust` in **both** modes, because without it gemini quietly
refuses to load project MCP servers at all, which would mean no bus.

**This is a real trade and it is on by default, so here it is plainly: an agent running like this
can reach anything you can.** Run Identra on work you have committed.

The reason it is the default is parallelism rather than convenience. An approval prompt is
reasonable once and unusable four times at once, which is what a board of parallel agents turns it
into. Worse, the orchestrator seat runs headless, so a prompt it stops on is a prompt nobody can
see: codex meets a folder it has not opened before with "Do you trust the contents of this
directory?" and waits there, and the instruction you typed goes into that menu instead of into the
work.

### Ask

Identra passes no autonomy flags at all and each CLI keeps its own defaults. Every edit and every
command waits for a click, and you answer the prompts in the node like any other terminal.

One switch in Settings, and it applies to the next agent you drop rather than to the ones already
running, because a CLI reads this at launch.

### What Identra deliberately does not pass

Claude Code has a `--strict-mcp-config` flag. It would guarantee the bus attaches with no prompt,
and it would do that by ignoring every MCP server you configured yourself. Taking your own tooling
away is not something to do quietly as a side effect of a permissions setting, so Identra does not
pass it.

## Connecting two agents

The connection is the permission. Not connected, no shared context.

Connect them **before** you launch the second one. A CLI reads its MCP servers once at startup, so
an agent that was already running when you connected it will not see the connection until it
restarts. Revoking is the other way round and takes effect immediately, because the bus re-reads
the connections on every call — a permission should be slower to give than to take away.

Connected agents can message each other, split work, and hand results back over the local bus, and
what they put on the shared task board shows up in the work panel while you watch.

Agents can connect themselves to each other with `connect_nodes`, which is a real grant and not a
formality. Every connection in a workspace is listed under **Links**, saying which ones you made
and which an agent made for itself, and any of them can be revoked there. Locking a tab stops
agents connecting anything to it at all.

## Your credentials

Identra never sees them. Each agent stays signed in through its own CLI config, exactly as it was
before Identra existed, and Identra neither stores nor forwards a key. If `claude --version` works
in your terminal and you are signed in there, Identra can run it.

## See also

- [Memory](./memory.md) — what a project remembers and how an agent reads it
- [Troubleshooting](./troubleshooting.md) — when an agent will not appear or will not start

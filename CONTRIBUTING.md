# Contributing

Bug reports are the most useful thing you can send. Identra runs other people's CLIs on other
people's machines, and most of what breaks is something I cannot reproduce here.

## Reporting something

Open an issue. The template asks for the four things that decide whether a report is actionable:
version, OS, how you installed, and which agent was running.

For anything visual — a terminal inside a node drawing wrongly, a layout that collapses, output that
looks corrupted — **send a screenshot**. What an agent drew is the bug. A description of it is a
description of a screenshot.

Security issues go through
[private advisories](https://github.com/identra-dev/identra/security/advisories/new) rather than
public issues. Identra gives agents a bus, a workspace boundary, and a task board; anything that
lets one reach past what it was wired to is worth reporting quietly first.

## Building it

```bash
just doctor   # says what is missing
just dev      # the app, with hot reload
just check    # fmt, clippy, and the whole test suite
```

You need Rust, `cargo-tauri`, and bun. On Linux you also need the webkit development packages the
README lists. `just doctor` will tell you which of those you are missing before you find out from a
linker error.

`just dev` clears its own port first, so an earlier run that died does not break the next one.

## Before you open a pull request

```bash
just check
```

That is `cargo fmt --check`, `cargo clippy -- -D warnings`, and the tests, which is exactly what CI
runs. Warnings are errors here, so a clean local run is a clean CI run.

The frontend has its own:

```bash
cd apps/identra-desktop/frontend && bun run build && bun test
```

## What the code expects of you

**Comments explain why, not what.** The code says what it does. A comment earns its place by saying
what it cost to learn: the case that broke, the approach that was tried and thrown away, the reason
the obvious version is wrong. Several of the comments in here are load-bearing for exactly that
reason, and a change that removes one usually reintroduces the bug it describes.

**A non-trivial change brings a test.** Not a suite, one test — the smallest thing that fails if the
logic breaks. Look at what is already there: most of them read as a sentence about the bug they
prevent, because the name is what someone sees when it goes red at 2am.

**Conventional commits**, with a subject that says what changed for a person using Identra rather
than which function moved.

## Where things are

| Path | What lives there |
|---|---|
| `crates/identra-core` | PTY and terminal manager, canvas, workspaces, agent detection, settings |
| `crates/identra-mcp` | The context bus: MCP server, tools, per-agent launch config |
| `crates/identra-memory` | SQLite store and the local embedding model |
| `apps/identra-desktop/src-tauri` | The shell: Tauri commands, window, app lifecycle |
| `apps/identra-desktop/frontend` | React canvas, nodes, command center |

Two things are worth knowing before you touch the bus. Agent-facing text is charged to every agent
on every canvas, once per session, so a paragraph added to the workspace guide is a bill someone
pays for the life of the product — there is a test that fails if it grows. And a tool's own
description is where "when should I use this" belongs, because it reaches the agent either way and
there it is paid once rather than twice.

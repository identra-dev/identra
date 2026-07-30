# Changelog

What changed in each release, in the terms a person using Identra would notice.

Identra updates itself from Releases, and the update strip asks before it installs anything. This
file is the answer to "what am I saying yes to".

## v0.1.2

**Every dialog closes the way you expect.** Escape closed the settings panel, the wallpaper picker,
the files panel and the work panel, and then did nothing in the two newest dialogs. Now it closes
all of them, from anywhere inside them.

- **Escape cancels the name-a-workspace dialog** even when focus has left the text field. It was
  bound to the input itself, so tabbing once to Create was enough to make the key stop working.
- **Escape and clicking away both dismiss the "what should I call you?" question.** It was the only
  overlay in the app with no way out but its own two buttons, on the first screen a new install
  shows. Dismissing means "not now" and asks again next launch; it deliberately does not save an
  answer, because "do not greet me" is a permanent choice and deserves the button rather than a
  keystroke someone pressed to make a box go away.

**Documentation you can read without cloning the repo.** `docs/` promised a UI spec, a contributing
guide and three diagrams, and contained none of them. It now holds what it says it does:

- **[Agents](docs/agents.md)** — the four CLIs Identra runs, the exact flag each one is passed in
  bypass mode, why Claude Code is the one that shows you project memory on connect, and why
  `--strict-mcp-config` is deliberately not passed.
- **[Memory](docs/memory.md)** — what a session leaves behind, why recall matches meaning rather
  than words, why there is no confidence threshold, and how to turn it off.
- **[Troubleshooting](docs/troubleshooting.md)** — the AppImage that needs libfuse2, the unnotarized
  dmg, an agent that is installed but invisible to a GUI launch, a wire drawn after launch, and port
  1420.

**The canvas and the memory panel say what they are actually doing.** Three things the app knew and
did not tell you, each of which read as the product being broken while it worked as designed.

- **A wire drawn onto an agent that is already running now says it is waiting.** An edge is the
  permission to share context and a CLI reads its MCP servers once at startup, so a wire drawn after
  launch was real, saved, and carried nothing until that node next started. It did that silently, so
  wiring two agents and watching nothing happen looked like a broken bus. The wire is now dashed and
  reads "connects at next launch", which is the truth for exactly as long as it holds.
- **The memory panel says which wording of a fact the agents are handed.** Three agents recording one
  decision in three sentences gave the agents one fact and the panel three rows, with nothing to say
  they were the same thing, so memory looked like it was losing track of itself. Older wordings are
  marked "restated more recently". Nothing is hidden and every row is still deletable, because a fact
  you cannot see is a fact you cannot correct.
- **A memory fact that differs only by a number is no longer thrown away.** "deploy the worker to
  region us-east-1" and the same line ending "us-east-2" scored as restatement and the second was
  dropped out of every agent's briefing, silently. Two deploy regions, two ports, two retry counts:
  figures are now compared before phrasing, and a disagreement about one settles it.

## v0.1.1

**Install with one command, and nothing downloads after it.**

```bash
curl -fsSL https://raw.githubusercontent.com/identra-dev/identra/main/install.sh | sh
```

It picks the build for your machine and installs it: the `.deb` where apt exists, the `.rpm` where
dnf does, the `.dmg` on macOS. Previously you read a releases page and chose between five files, and
on Linux the obvious one did not start — an AppImage mounts itself with libfuse2 and current Ubuntu
and Fedora ship only fuse3. The AppImage is still there as a fallback and is now unpacked at install
time, so that failure cannot happen either.

**The embedding model ships inside the build.** Recall matches on meaning rather than on shared
words, and it does that on first launch, offline, with nothing to wait for. It used to arrive as a
130MB download landing on your very first memory. The download is paid once now, at install time.

### Fixed

- **A message sent to an agent presses enter for itself.** Typing an instruction in the command bar
  filled the agent's prompt and stopped there, waiting for you to press enter; a message handed
  between two wired agents did the same, once per handoff. Body and return were being written
  together, and an agent CLI reads a single chunk as a paste, where a return means a new line rather
  than send.
- **Closing the app closes it.** The window went and the process stayed, and the only way out was a
  process list. The agents go with it now, rather than being left running invisibly.
- **One pty has one size.** A node can be open on the canvas, at full window, and in the command
  center at once, and each was re-wrapping the agent's output for the others — which looks like the
  text being corrupted rather than like two views disagreeing. Read-only views no longer reflow what
  everyone else is reading, and closing the full-window view hands the size back.
- **The command center shows a real terminal**, so what the orchestrator drew is what you read.
- **Renaming a workspace you opened from your own folders leaves the folder where it is.** It used
  to move it.
- **The orchestrator seat goes to an agent that reads its brief.**

### Changed

- **Agents cost about a third of what they did to start.** Every agent was being handed roughly
  3,100 tokens before you typed anything, most of it repeating what its own tools already said about
  themselves. Now about 950, with nothing removed that an agent cannot read off the tool it is about
  to call.
- **Everything starts in bypass mode and connects to the context bus without a prompt**, so a node
  is working the moment it opens instead of waiting on two dialogs.
- **The first-run screen leads with Claude Code.** A project's memory reaches an agent over the MCP
  handshake, and that is the CLI which surfaces what arrives there, so it is the one where an agent
  opens already knowing what the project has learned. codex, gemini, and opencode all still run.

## v0.1.0

First release. Canvas of agent nodes, each a real CLI in a real terminal; a local context bus they
talk over; a shared task board; per-project memory; workspaces backed by ordinary folders.

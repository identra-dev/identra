# Changelog

What changed in each release, in the terms a person using Identra would notice.

Identra updates itself from Releases, and the update strip asks before it installs anything. This
file is the answer to "what am I saying yes to".

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

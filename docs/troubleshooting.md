# Troubleshooting

The failures people actually hit, and what each one really means. If something here is wrong or
missing, an issue is welcome.

## Installing

### The AppImage will not start

It needs `libfuse2`, and current Ubuntu and Fedora ship only fuse3. An AppImage mounts itself, so
without it nothing happens and the error, if you get one, does not say that.

Use the `.deb` or the `.rpm` next to it on the releases page. Both are unpacked at install time and
cannot hit this. The install script prefers them for exactly this reason:

```bash
curl -fsSL https://raw.githubusercontent.com/identra-dev/identra/main/install.sh | sh
```

### macOS says it cannot check the app for malicious software

The `.dmg` is not notarized yet. Either:

```bash
xattr -cr /Applications/Identra.app
```

or right-click the app and choose Open, which offers the same dialog with an Open button on it.

### The `.dmg` will not run on my Intel Mac

It is built for Apple Silicon, and Intel is not supported yet. The local embedding model's runtime
ships no x86 macOS build, and shipping a `.dmg` that half works would be worse than saying so.

### Windows

Not supported yet. It is not a small port: interactive agent TUIs need ConPTY, the paths differ, and
the webview wants its own testing.

## Agents

### "No coding agent found" and I have one installed

Identra looks on your `PATH`. The case that has actually shipped broken is a GUI launch: an app
started from a Dock, a launcher or Finder gets a stripped `PATH`, so a CLI that resolves perfectly
in your terminal is invisible to the app.

Check what Identra sees under that stripped environment:

```bash
just detect-check
```

Every agent you have installed should print an absolute command. If one is missing there but works
in your shell, it is installed somewhere only your shell's startup files know about.

The first-run panel has a **Check again** button; it clears the probe cache and re-detects, so you
do not have to relaunch after installing something.

### An agent opens but is not connected to the others

Draw the wire first, then launch. A CLI reads its MCP servers once at startup, so an edge drawn
after an agent is already running does nothing until that node restarts.

### gemini will not load the bus

It needs `--skip-trust`, which Identra passes in both autonomy modes. Without it gemini quietly
refuses project MCP servers entirely. If you are launching gemini by hand outside Identra, that is
the flag you are missing.

### An agent is waiting and I cannot see what it wants

A node in the needs-input state is asking something. Open it full-window: reading a conversation in
a tile is squinting, and the same terminal is there at full size.

If you would rather not be asked at all, Settings has one switch. See
[Agents](./agents.md#what-an-agent-may-do-without-asking) for exactly what it turns off.

## Memory

### Recall is matching words, not meaning

Either `IDENTRA_EMBEDDINGS=off` is set, or the model is not loaded. The memory panel says which,
and a failure there offers a retry.

A build from source has no bundled model and fetches one on first use; a release build carries its
own and works offline immediately.

## The app itself

### The window will not close

It should, from the title bar, from the quit control, or with `Ctrl+Q` / `Cmd+Q`. If it refuses,
Identra now says so on screen instead of failing silently, and names what went wrong.

Closing flushes your canvas first, but that flush is bounded: the window goes whether or not the
save lands, because an app that will not close over a write you cannot see is holding you hostage.

### `just dev` fails naming port 1420

Something is still holding it. Vite is pinned to 1420 with `strictPort` because the window loads
that exact address, and a dev server that quietly moved to 1421 gives you a blank window instead of
an app.

An interrupted `just dev` can leave the built app, vite and cargo-tauri behind, and then every later
run fails the same way. `just dev` clears the port itself now, but if you are stuck, kill whatever
holds 1420 and any stray cargo processes, then try again.

A worse version of the same thing: a leftover dev server plus a fresh relaunch gets you the **old**
backend under a hot-reloaded new frontend, which looks like the app being haunted rather than like
two processes disagreeing.

### The canvas came back empty

The canvas saves itself to `.identra/canvas.json` in the workspace with a debounced atomic write. If
that file will not parse it is moved aside to `.bak` rather than discarded, so nothing is destroyed
and the previous state is still on disk to look at.

### Identra wrote files into my repository

It does, and the full list is in the [README](../README.md#what-identra-writes-into-your-project).
The three worth knowing about are `.mcp.json`, `.gemini/settings.json` and `AGENTS.md` /
`CLAUDE.md`, because those are often already yours. Identra **merges** into the config files rather
than replacing them: it inserts or replaces its own `identra-bus` key and writes everything else
back untouched. Re-opening a folder is a fixed point, so nothing grows and nothing churns.

## Building from source

Run this first; it names what is missing rather than failing halfway through a build:

```bash
just doctor
```

On Debian and Ubuntu the webview build dependencies are the usual gap:

```bash
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

macOS renders through WKWebView, which is already there.

If `just build` fails on the frontend step when you invoked `cargo tauri build` yourself, run it
from `apps/identra-desktop` — the frontend build script it shells out to lives there, not at the
repo root.

## Still stuck

Open an issue at
[github.com/identra-dev/identra/issues](https://github.com/identra-dev/identra/issues). What helps
most: your OS, how you installed, which agent, and what the terminal in the node said.

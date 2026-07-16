# identra-core

The Rust engine, as a library. No UI, no I/O framework lock-in — `identra-desktop` (the Tauri
shell) and `identra-mcp` build on top of it.

It owns the hard parts: a PTY/terminal manager that spawns a real agent CLI per node, tags each
output chunk with a sequence number, and keeps a ring buffer so a node reattaches cleanly after a
reload; detection of which agent CLIs are installed and signed in (by stat only, it never reads a
credential); and the canvas store that persists the board to `.identra/canvas.json` with an
atomic write.

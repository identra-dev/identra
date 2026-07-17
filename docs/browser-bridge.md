# The browser node and the agent bridge

The browser node is a web view on the canvas: an iframe pointed at a URL (your dev server, say),
with a Yaru URL bar in the header. The URL rides in the node's `cwd` field, so it saves and
reloads with the canvas like any other node. This half ships today and needs no extra tooling.

## Letting an agent read the page

The second half, an agent that reads the rendered page and fixes the code, has a hard platform
constraint. On Linux, Tauri renders through webkitgtk, which is not driveable over the Chrome
DevTools Protocol and exposes no screenshot API to the app. So there is no in-process way to hand
an agent a picture of what the browser node is showing. Two real paths exist, and both sit outside
what the webkitgtk webview can do on its own:

- **Chromium plus chrome-devtools-mcp (the drive path).** With Chromium installed, register
  `chrome-devtools-mcp` in the agent's config before launch (same startup-registration rule as the
  context bus). The agent then drives a real Chrome at the same URL for live DOM, console, and
  clicks. This needs Chromium on the machine, which is why it is an opt-in upgrade, not the default.
- **An OS-level capture (the fallback).** A screenshot of the node's screen region written to a
  file the agent reads. It is fragile: it depends on the display server (X11 vs Wayland), on a
  capture tool being present, and on mapping a React Flow node to screen coordinates. Not worth
  shipping as a core feature when the drive path above is both cleaner and more capable.

Until one of those is wired, the browser node is the human's window on the work: you watch the live
preview hot-reload while the agent edits code, and the agent works from what you tell it or from the
context bus. That is the honest state, and it is enough for the single-machine flows Identra targets
today.

// Attaching a live xterm to a PTY the engine already has open.
//
// Three places show one node's output now: the node on the canvas, the full-window focus view, and
// the command center pane. The canvas node owns spawning and the status dot, so it keeps its own
// effect; the other two only ever attach to something already running, and that is this.
//
// The attach itself is the part worth having once. Ask for the snapshot, write it, then let the
// live stream through from where the snapshot ended, so a reader that arrives mid-session sees the
// conversation so far and then follows it without dropping or repeating a line.
import { useEffect, type RefObject } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import {
  onExit,
  onOutput,
  terminalInput,
  terminalResize,
  terminalSnapshot,
  type OutputEvent,
} from "./api";
import { pastSnapshot } from "./reattach";

/// Fired when a view that was driving a pty's size lets go of it, so whatever is still showing that
/// node can claim the size back.
///
/// A pty has one size and can have three views. The canvas node is the one that outlives the
/// others, but it is also the one that cannot notice: the focus view is an overlay, so opening and
/// closing it never changes the node's own box and never trips its ResizeObserver. Without a nudge
/// the node is left wrapped for a window that is no longer there.
export const REFIT_EVENT = "identra:refit";

export type AttachOptions = {
  // What to call the agent in the line printed when it exits.
  kind: string;
  fontSize: number;
  // Whether keystrokes reach the pty. The command center pane is a window onto the orchestrator,
  // not a second place to type at it: the bar under it is where instructions go, and two inputs
  // onto one agent is how you end up with half a sentence in each.
  readOnly?: boolean;
  // Take focus once attached. The focus view wants it, a pane inside a form does not.
  focusOnAttach?: boolean;
  // Swallowed before the pty sees them, so a view can keep a key for itself. Return true to keep
  // the key, which mirrors xterm's own handler contract.
  onKeyEvent?: (e: KeyboardEvent) => boolean;
  // Said instead of a blank rectangle when there is nothing behind this id.
  emptyMessage?: string;
};

/// Attach a terminal to `nodeId`'s PTY inside `host`, for as long as the component is mounted.
///
/// Teardown is frontend only: the PTY keeps running, which is what lets the same node be open on
/// the canvas and at full size at once, and what lets a view be closed and reopened without
/// costing the agent its session.
export function useAttachedTerminal(
  host: RefObject<HTMLDivElement | null>,
  nodeId: string | null,
  opts: AttachOptions,
) {
  const {
    kind,
    fontSize,
    readOnly = false,
    focusOnAttach = false,
    onKeyEvent,
    emptyMessage = "This node is not running.",
  } = opts;

  useEffect(() => {
    const el = host.current;
    if (el === null || nodeId === null) return;

    const term = new Terminal({
      fontFamily: "Ubuntu Mono, Menlo, Consolas, monospace",
      fontSize,
      cursorBlink: !readOnly,
      // A reader scrolls back through what an agent said; a node that repaints does not need it.
      // Cheap either way, and the alternative is losing the top of a long reply.
      scrollback: 5000,
      theme: {
        background: "#300a24",
        foreground: "#ffffff",
        cursor: "#e95420",
      },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    if (onKeyEvent !== undefined) term.attachCustomKeyEventHandler(onKeyEvent);
    term.open(el);
    fit.fit();

    let lastSeq = 0;
    let ready = false;
    let disposed = false;
    const buffered: OutputEvent[] = [];
    const write = (e: OutputEvent) => {
      if (pastSnapshot(e.seq, lastSeq)) {
        term.write(new Uint8Array(e.data));
        lastSeq = e.seq;
      }
    };
    const unlisten = onOutput((e) => {
      if (e.id !== nodeId) return;
      if (ready) write(e);
      else buffered.push(e);
    });
    const unlistenExit = onExit((e) => {
      if (e.id !== nodeId) return;
      const how = e.code === null ? "was stopped" : `exited (${e.code})`;
      term.write(`\r\n\x1b[90m${kind} ${how}\x1b[0m\r\n`);
    });

    void (async () => {
      const snap = await terminalSnapshot(nodeId);
      if (disposed) return;
      if (snap === null) {
        // Nothing behind this id: it died between the click and the attach, or it never started.
        // Say so rather than showing a void.
        term.write(`\r\n\x1b[90m${emptyMessage}\x1b[0m\r\n`);
      } else {
        term.write(new Uint8Array(snap.data));
        lastSeq = snap.lastSeq;
      }
      for (const e of buffered) write(e);
      buffered.length = 0;
      ready = true;
      if (focusOnAttach) term.focus();
    })();

    const onData = readOnly
      ? null
      : term.onData(
          (d) =>
            void terminalInput(nodeId, d).catch((err) =>
              console.warn(`input to ${nodeId} dropped:`, err),
            ),
        );

    // The local xterm always re-fits, so what is on screen here is laid out for the box it is in.
    // Whether that reaches the pty is a different question, and the answer is: only if this view is
    // one you can type into.
    //
    // There is one pty per node and up to three views onto it — the canvas node, the focus view,
    // and the command center pane. The pty has a single size, so every view that pushes its own is
    // re-wrapping the agent's output for all the others. A 12px read-only pane a few hundred pixels
    // wide would drag the CLI down to its own column count, and the node showing the same agent at
    // full width would then be drawing text wrapped for a box it is not in. With both mounted the
    // two observers take turns, and the agent redraws its whole TUI at a new width each time, which
    // looks exactly like the output being corrupted.
    //
    // A spectator does not get to reflow what everyone else is reading. Read-only means read only.
    const ro = new ResizeObserver(() => {
      try {
        fit.fit();
        // A resize racing a killed terminal rejects at the backend with nothing to say to anyone,
        // so it is swallowed rather than left unhandled.
        if (!readOnly) void terminalResize(nodeId, term.rows, term.cols).catch(() => {});
      } catch {
        /* host detached mid-resize */
      }
    });
    ro.observe(el);

    return () => {
      disposed = true;
      ro.disconnect();
      onData?.dispose();
      void unlisten.then((un) => un());
      void unlistenExit.then((un) => un());
      term.dispose();
    };
  }, [
    host,
    nodeId,
    kind,
    fontSize,
    readOnly,
    focusOnAttach,
    onKeyEvent,
    emptyMessage,
  ]);
}

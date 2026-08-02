// Attaching a live xterm to a PTY the engine already has open.
//
// Two places show a node's output: a pane in the centre column, and the command center's own strip
// under the bar. A pane may also be the thing that starts the process, which is what `start` is for;
// the command center never is, because the seat is spawned before any view of it exists.
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

// The refit event that used to live here is gone with the overlay that needed it. A pane is the
// view now, and a pane changing size is a box changing size, which its own ResizeObserver already
// sees. Nothing has to be told.

export type AttachOptions = {
  // What to call the agent in the line printed when it exits.
  kind: string;
  fontSize: number;
  // Whether keystrokes reach the pty. The command center pane is a window onto the orchestrator,
  // not a second place to type at it: the bar under it is where instructions go, and two inputs
  // onto one agent is how you end up with half a sentence in each.
  readOnly?: boolean;
  // Take focus once attached. A pane the user just opened wants it; a strip inside a form does not.
  focusOnAttach?: boolean;
  // Swallowed before the pty sees them, so a view can keep a key for itself. Return true to keep
  // the key, which mirrors xterm's own handler contract.
  onKeyEvent?: (e: KeyboardEvent) => boolean;
  // Said instead of a blank rectangle when there is nothing behind this id.
  emptyMessage?: string;
  // Called when there is nothing behind this id, instead of printing `emptyMessage`. This is what
  // makes a view able to own a node rather than only spectate on one: a pane that mounts on a node
  // nobody has started is the thing that starts it, at the size it is actually being drawn at.
  //
  // It returns the line to print, already ANSI-formatted, or null when the process started and the
  // terminal should stay clean. Reporting through the return rather than by throwing is deliberate:
  // "that agent is not installed" is an ordinary answer with somewhere to go, and the place it
  // belongs is the black rectangle the user is already looking at.
  start?: (rows: number, cols: number) => Promise<string | null>;
  // Every byte written to the terminal, snapshot replay included. A dev server's address is in its
  // banner, and on a reattach that banner is in the snapshot rather than in any chunk still to come,
  // so a watcher that only saw the live stream would never find it.
  onBytes?: (bytes: Uint8Array) => void;
};

/// Attach a terminal to `nodeId`'s PTY inside `host`, for as long as the component is mounted.
///
/// Teardown is frontend only: the PTY keeps running, which is what lets one node be open in two
/// panes at once, and what lets a pane be closed and reopened without costing the agent its
/// session.
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
    start,
    onBytes,
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
        // Matches the pane background in styles.css. Ubuntu aubergine at a third of its chroma:
        // at full saturation it was tuned for a neutral grey desktop, and inside an aubergine
        // frame every pane read as a purple block.
        background: "#1d1019",
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
        const bytes = new Uint8Array(e.data);
        term.write(bytes);
        onBytes?.(bytes);
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
        // Nothing behind this id: it died between the click and the attach, it never started, or
        // this view is the one whose job it is to start it.
        if (start === undefined) {
          term.write(`\r\n\x1b[90m${emptyMessage}\x1b[0m\r\n`);
        } else {
          const failed = await start(term.rows, term.cols);
          if (disposed) return;
          if (failed !== null) term.write(failed);
        }
      } else {
        const bytes = new Uint8Array(snap.data);
        term.write(bytes);
        onBytes?.(bytes);
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
    // There is one pty per node and more than one view onto it: a pane, possibly a second pane in
    // the other half of a split, and the command center's strip. The pty has a single size, so every
    // view that pushes its own is re-wrapping the agent's output for all the others. A read-only
    // strip a few hundred pixels wide would drag the CLI down to its own column count, and a pane
    // showing the same agent at full width would then be drawing text wrapped for a box it is not
    // in. With both mounted the two observers take turns, and the agent redraws its whole TUI at a
    // new width each time, which looks exactly like the output being corrupted.
    //
    // A spectator does not get to reflow what everyone else is reading. Read-only means read only.
    const ro = new ResizeObserver(() => {
      try {
        fit.fit();
        // A resize racing a killed terminal rejects at the backend with nothing to say to anyone,
        // so it is swallowed rather than left unhandled.
        if (!readOnly)
          void terminalResize(nodeId, term.rows, term.cols).catch(() => {});
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
    start,
    onBytes,
  ]);
}

// One node's conversation at full window size. A second view of the same PTY, not a second
// conversation: it attaches the way a reloaded node does (snapshot, then the stream from where
// the snapshot ends) and types into the same pipe, so the canvas node behind it stays live and
// nothing is moved or paused to look at it.
import { useEffect, useRef } from "react";
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
import { AgentIcon } from "./icons";

type Props = {
  nodeId: string;
  title: string;
  kind: string;
  onClose: () => void;
};

export default function FocusView({ nodeId, title, kind, onClose }: Props) {
  const host = useRef<HTMLDivElement>(null);
  // Through a ref so the terminal's key handler, wired once, always calls the current one.
  const close = useRef(onClose);
  close.current = onClose;

  // Ctrl+Esc, not Esc. Plain Esc belongs to whatever runs in the terminal: the agent TUIs bind
  // it (cancel the composer, interrupt) and so does vim, so a close key of Esc would make
  // leaving this view also poke the conversation. Driving the first build proved it: the byte
  // landed in the PTY as ^[ and the view stayed open.
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape" && e.ctrlKey) onClose();
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [onClose]);

  // The attach dance is AgentNode's, trimmed: no spawn (a node that never launched has nothing
  // to show at full size and the button does not render), no status heuristics (the canvas node
  // keeps owning the dot), just snapshot, gated stream, input, resize.
  useEffect(() => {
    const el = host.current;
    if (!el) return;
    const term = new Terminal({
      fontFamily: "Ubuntu Mono, Menlo, Consolas, monospace",
      fontSize: 14,
      cursorBlink: true,
      theme: {
        background: "#300a24",
        foreground: "#ffffff",
        cursor: "#e95420",
      },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    // The same rule as the window listener, but inside the terminal, where keydown never
    // bubbles out: Ctrl+Esc closes and is swallowed, everything else reaches the agent.
    term.attachCustomKeyEventHandler((e) => {
      if (e.type === "keydown" && e.key === "Escape" && e.ctrlKey) {
        close.current();
        return false;
      }
      return true;
    });
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
        // The agent died between the click and the attach. Say so rather than showing a void.
        term.write("\r\n\x1b[90mThis node is not running.\x1b[0m\r\n");
      } else {
        term.write(new Uint8Array(snap.data));
        lastSeq = snap.lastSeq;
      }
      for (const e of buffered) write(e);
      buffered.length = 0;
      ready = true;
      term.focus();
    })();

    const onData = term.onData(
      (d) =>
        void terminalInput(nodeId, d).catch((err) =>
          console.warn(`input to ${nodeId} dropped:`, err),
        ),
    );
    // The resize follows this view while it is open. When it closes, the canvas node's own
    // ResizeObserver fires on nothing changing? No: the PTY keeps the focus size until the node
    // is next resized. That is visible as rewrapping and is the honest cost of two views over
    // one terminal; the node re-fits itself the next time its own size changes.
    const ro = new ResizeObserver(() => {
      try {
        fit.fit();
        void terminalResize(nodeId, term.rows, term.cols).catch(() => {});
      } catch {
        /* host detached mid-resize */
      }
    });
    ro.observe(el);

    return () => {
      disposed = true;
      ro.disconnect();
      onData.dispose();
      void unlisten.then((un) => un());
      void unlistenExit.then((un) => un());
      term.dispose();
    };
  }, [nodeId, kind]);

  return (
    <div className="identra-focus">
      <div className="identra-focus__bar">
        <AgentIcon kind={kind} className="identra-node__icon" />
        <span className="identra-focus__title">{title || kind}</span>
        <span className="identra-focus__hint">Ctrl+Esc closes</span>
        <button
          className="identra-focus__close"
          title="Back to the canvas"
          onClick={onClose}
        >
          &times;
        </button>
      </div>
      <div className="identra-focus__term" ref={host} />
    </div>
  );
}

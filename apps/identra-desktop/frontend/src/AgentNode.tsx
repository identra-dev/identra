import { memo, useEffect, useRef, useState } from "react";
import { Handle, Position, type NodeProps } from "@xyflow/react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import {
  agentsByKind,
  onOutput,
  terminalInput,
  terminalResize,
  terminalSnapshot,
  terminalStart,
  type OutputEvent,
} from "./api";
import { pastSnapshot } from "./reattach";
import { iconFor } from "./icons";

// `kind` is the agent id (codex, claude, …); the node resolves its binary and args from it.
export type AgentNodeData = { title: string; cwd: string | null; kind: string };

function AgentNodeImpl({ id, data }: NodeProps) {
  const nodeData = data as AgentNodeData;
  const termHost = useRef<HTMLDivElement>(null);
  // Honest liveness with no engine change: the node already hears every terminal://output chunk.
  // A chunk flips the dot green; 1.5s of silence lets it settle back to idle orange.
  const [state, setState] = useState<"ready" | "running">("ready");

  // One terminal per node, wired to the backend PTY. Runs once per mount; on a hot reload it
  // reattaches to the still-running PTY instead of restarting it.
  useEffect(() => {
    const host = termHost.current;
    if (!host) return;

    const term = new Terminal({
      fontFamily: "Ubuntu Mono, Menlo, Consolas, monospace",
      fontSize: 13,
      cursorBlink: true,
      theme: {
        background: "#300a24",
        foreground: "#ffffff",
        cursor: "#e95420",
      },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host);
    fit.fit();

    let lastSeq = 0;
    let ready = false; // until the snapshot is applied, buffer live chunks
    let disposed = false;
    const buffered: OutputEvent[] = [];

    let running = false;
    let idleTimer: number | undefined;
    const markOutput = () => {
      if (!running) {
        running = true;
        setState("running");
      }
      window.clearTimeout(idleTimer);
      idleTimer = window.setTimeout(() => {
        running = false;
        setState("ready");
      }, 1500);
    };

    const write = (e: OutputEvent) => {
      if (pastSnapshot(e.seq, lastSeq)) {
        term.write(new Uint8Array(e.data));
        lastSeq = e.seq;
      }
    };

    const unlisten = onOutput((e) => {
      if (e.id !== id) return;
      markOutput();
      if (ready) write(e);
      else buffered.push(e);
    });

    void (async () => {
      const snap = await terminalSnapshot(id);
      if (disposed) return;
      if (snap === null) {
        // Fresh node, or the app was fully restarted: launch this node's CLI now. Resolve the
        // binary from the agent registry by kind so each node runs its own agent, not codex.
        const agent = (await agentsByKind()).get(nodeData.kind);
        if (disposed) return;
        if (!agent || !agent.available) {
          term.write(
            `\r\n\x1b[31m${nodeData.kind} isn't installed\x1b[0m or not on your PATH.\r\n`,
          );
          term.write(
            "Run `just doctor` to see what's missing, then reopen the node.\r\n",
          );
        } else {
          try {
            await terminalStart(
              id,
              agent.cmd,
              agent.args,
              nodeData.cwd,
              term.rows,
              term.cols,
            );
          } catch (err) {
            term.write(
              `\r\n\x1b[31m${agent.name} didn't start:\x1b[0m ${err}\r\n`,
            );
          }
        }
      } else {
        term.write(new Uint8Array(snap.data));
        lastSeq = snap.lastSeq;
      }
      for (const e of buffered) write(e); // drain what arrived during the await
      buffered.length = 0;
      ready = true;
    })();

    const onData = term.onData((d) => void terminalInput(id, d));

    const ro = new ResizeObserver(() => {
      try {
        fit.fit();
        void terminalResize(id, term.rows, term.cols);
      } catch {
        /* host detached mid-resize */
      }
    });
    ro.observe(host);

    return () => {
      disposed = true;
      window.clearTimeout(idleTimer);
      ro.disconnect();
      onData.dispose();
      void unlisten.then((un) => un());
      term.dispose();
      // Frontend teardown only — the backend PTY stays alive so a reload can
      // reattach. App.onNodesDelete is what actually kills it.
    };
  }, [id]);

  const icon = iconFor(nodeData.kind);
  return (
    <div className="identra-node">
      <Handle type="target" position={Position.Left} className="identra-port" />
      <Handle type="source" position={Position.Right} className="identra-port" />
      <div className="identra-node__header">
        <span className="identra-node__dot" data-state={state} />
        <span className="identra-node__icon" style={{ background: icon.tile }}>
          {icon.glyph}
        </span>
        <span className="identra-node__title">
          {nodeData.title || nodeData.kind}
        </span>
      </div>
      <div className="identra-node__term nodrag nowheel" ref={termHost} />
    </div>
  );
}

export default memo(AgentNodeImpl);

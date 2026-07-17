import { memo, useEffect, useRef, useState, type CSSProperties } from "react";
import {
  Handle,
  Position,
  useReactFlow,
  type NodeProps,
} from "@xyflow/react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import {
  agentsByKind,
  onExit,
  onOutput,
  terminalInput,
  terminalResize,
  terminalSnapshot,
  terminalStart,
  type OutputEvent,
} from "./api";
import { pastSnapshot } from "./reattach";
import { AgentIcon, auraFor } from "./icons";

// `kind` is the agent id (codex, claude, …); the node resolves its binary and args from it.
export type AgentNodeData = { title: string; cwd: string | null; kind: string };

function AgentNodeImpl({ id, data }: NodeProps) {
  const nodeData = data as AgentNodeData;
  const { deleteElements } = useReactFlow();
  const termHost = useRef<HTMLDivElement>(null);
  // Three honest states. Output means it is working; 1.5s of quiet settles it back to ready. Exit
  // is the only one the node cannot infer for itself, so the engine tells it: without that, an
  // agent that finished looks exactly like one that is thinking, forever.
  const [state, setState] = useState<"ready" | "running" | "exited">("ready");

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
    let exited = false;
    let idleTimer: number | undefined;
    const markOutput = () => {
      if (exited) return; // a dead agent producing bytes is drain, not life
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

    const unlistenExit = onExit((e) => {
      if (e.id !== id) return;
      window.clearTimeout(idleTimer); // it cannot go back to running now
      running = false;
      exited = true;
      setState("exited");
      // Say so in the terminal too. The dot tells you at a glance across the canvas; this tells you
      // why when you look, and a non-zero code is the difference between finished and crashed.
      const how = e.code === null ? "was stopped" : `exited (${e.code})`;
      term.write(`\r\n\x1b[90m${nodeData.kind} ${how}\x1b[0m\r\n`);
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
              nodeData.kind,
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
      void unlistenExit.then((un) => un());
      term.dispose();
      // Frontend teardown only: the backend PTY stays alive so a reload can
      // reattach. App.onNodesDelete is what actually kills it.
    };
  }, [id]);

  return (
    <div
      className="identra-node"
      data-state={state}
      style={{ "--aura": auraFor(nodeData.kind) } as CSSProperties}
    >
      <Handle type="target" position={Position.Left} className="identra-port" />
      <Handle
        type="source"
        position={Position.Right}
        className="identra-port"
      />
      <div className="identra-node__header">
        <span className="identra-node__dot" data-state={state} />
        <AgentIcon kind={nodeData.kind} className="identra-node__icon" />
        <span className="identra-node__title">
          {nodeData.title || nodeData.kind}
        </span>
        <button
          className="identra-node__close nodrag"
          title={`Close this ${nodeData.title || nodeData.kind}`}
          onClick={() => {
            // Naming what it is, because "delete this node" does not tell you that a conversation
            // goes with it. This is the only way to remove one now that the key does not.
            const name = nodeData.title || nodeData.kind;
            if (
              window.confirm(
                `Close ${name}?\n\nThe agent stops and its conversation is forgotten.`,
              )
            ) {
              void deleteElements({ nodes: [{ id }] });
            }
          }}
        >
          &times;
        </button>
      </div>
      <div className="identra-node__term nodrag nowheel" ref={termHost} />
    </div>
  );
}

export default memo(AgentNodeImpl);

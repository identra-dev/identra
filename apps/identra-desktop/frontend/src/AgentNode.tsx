import { memo, useEffect, useRef, useState, type CSSProperties } from "react";
import { Handle, Position, useReactFlow, type NodeProps } from "@xyflow/react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import {
  agentsByKind,
  devCommand,
  memoryList,
  onExit,
  onOutput,
  terminalInput,
  terminalResize,
  terminalSnapshot,
  terminalStart,
  terminalStatus,
  type Memory,
  type OutputEvent,
} from "./api";
import { appendTail, findLocalUrl } from "./devurl";
import { pastSnapshot } from "./reattach";
import { AgentIcon, auraFor } from "./icons";

// `kind` is the agent id (codex, claude, …); the node resolves its binary and args from it.
// `seat` is stamped on at render by App when this node holds the orchestrator seat. It is not
// persisted here: the canvas stores one seat id, and this is that fact arriving where it is drawn.
export type AgentNodeData = {
  title: string;
  cwd: string | null;
  kind: string;
  locked?: boolean;
  seat?: boolean;
  // Stamped on by App along with `seat`. A stable callback, so it does not break the memo on this
  // component every time App re-renders for something unrelated.
  onToggleLock?: (id: string) => void;
  // Stamped on the same way. A dev-server node calls it when the user clicks the address badge,
  // and App answers by standing a browser node up next to it.
  onOpenPreview?: (id: string, url: string) => void;
};

function AgentNodeImpl({ id, data }: NodeProps) {
  const nodeData = data as AgentNodeData;
  const { deleteElements } = useReactFlow();
  const termHost = useRef<HTMLDivElement>(null);
  // Four honest states. Output means it is working; 1.5s of quiet settles it. Exit is the only one
  // the node cannot infer for itself, so the engine tells it: without that, an agent that finished
  // looks exactly like one that is thinking, forever. And when it settles, quiet splits in two:
  // finished, or waiting on an answer. That last one the engine works out from the transcript,
  // because the difference is in what was printed, not in the timing.
  const [state, setState] = useState<
    "ready" | "running" | "needs-input" | "exited"
  >("ready");
  // What the project already knows, shown once when the node opens. This is the payoff made
  // visible: the agent has not typed a word and the human can already see it is not starting cold.
  // A few facts, not the whole store, because it is a glance and not the memory panel.
  const [recall, setRecall] = useState<Memory[]>([]);
  const [recallShown, setRecallShown] = useState(true);
  // A dev-server node runs the project's own dev command instead of an agent CLI. Same PTY, same
  // terminal, same lifecycle; what differs is where the command comes from and that its output is
  // watched for the preview address.
  const isDev = nodeData.kind === "dev";
  const [previewUrl, setPreviewUrl] = useState<string | null>(null);

  useEffect(() => {
    // A dev server has no conversation to remember; the recall strip on it would be noise.
    if (isDev) return;
    let dropped = false;
    void memoryList(3).then((facts) => {
      if (!dropped) setRecall(facts);
    });
    return () => {
      dropped = true;
    };
  }, [isDev]);

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

    // The preview address is fished out of the dev server's own banner. A rolling tail, because
    // chunk boundaries land anywhere, including mid-url.
    let urlTail = "";
    let urlFound = false;
    const decoder = new TextDecoder();
    const scanForUrl = (bytes: Uint8Array) => {
      if (!isDev || urlFound) return;
      urlTail = appendTail(urlTail, decoder.decode(bytes, { stream: true }));
      const url = findLocalUrl(urlTail);
      if (url !== null) {
        urlFound = true;
        setPreviewUrl(url);
      }
    };

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
        // Settling is the one moment worth asking the engine what this quiet means. I set ready
        // first and correct to needs-input after, so the node never waits on IPC to stop looking
        // busy. Anything other than needs-input leaves it as it is: the engine uses the same 1.5s
        // threshold, so it can still say "running" here by a hair, and the local timer is the one
        // that should win that tie.
        void terminalStatus(id)
          .then((status) => {
            if (disposed || exited || running) return;
            if (status === "needs-input") setState("needs-input");
          })
          .catch(() => {
            // The node was killed between settling and asking. Ready is already right.
          });
      }, 1500);
    };

    const write = (e: OutputEvent) => {
      if (pastSnapshot(e.seq, lastSeq)) {
        const bytes = new Uint8Array(e.data);
        term.write(bytes);
        scanForUrl(bytes);
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
        // Fresh node, or the app was fully restarted: launch this node's command now. An agent
        // node resolves its CLI from the registry by kind; a dev node asks the engine what this
        // project's dev command is.
        if (isDev) {
          const cmd = await devCommand();
          if (disposed) return;
          if (cmd === null || cmd.length === 0) {
            term.write(
              "\r\n\x1b[31mThis project does not declare a dev command\x1b[0m in package.json, a justfile, or a Makefile.\r\n",
            );
          } else {
            try {
              await terminalStart(
                id,
                nodeData.kind,
                cmd[0]!,
                cmd.slice(1),
                nodeData.cwd,
                term.rows,
                term.cols,
              );
            } catch (err) {
              term.write(
                `\r\n\x1b[31mThe dev server didn't start:\x1b[0m ${err}\r\n`,
              );
            }
          }
        } else {
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
        }
      } else {
        const bytes = new Uint8Array(snap.data);
        term.write(bytes);
        // The reattach replay holds the banner that was printed before the reload, so the URL is
        // in there, not in any chunk still to come.
        scanForUrl(bytes);
        lastSeq = snap.lastSeq;
      }
      for (const e of buffered) write(e); // drain what arrived during the await
      buffered.length = 0;
      ready = true;
    })();

    // A keystroke sent to an agent that has already exited fails at the pipe, and that is expected
    // for a dead node, so it drops to one warning rather than an unhandled rejection. The exit line
    // already told the user the agent is gone; there is nothing more to say and nothing to type into.
    const onData = term.onData(
      (d) =>
        void terminalInput(id, d).catch((err) =>
          console.warn(`input to ${id} dropped:`, err),
        ),
    );

    const ro = new ResizeObserver(() => {
      try {
        fit.fit();
        // Same story as input: a resize racing a killed terminal rejects at the backend, and there
        // is nothing to resize and nothing to tell the user, so it is swallowed rather than left
        // unhandled.
        void terminalResize(id, term.rows, term.cols).catch(() => {});
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
        {nodeData.seat === true && (
          // Named, not just coloured. A ring alone would say this node is special without saying
          // why, and the thing worth knowing is that the command bar types in here.
          <span
            className="identra-node__seat"
            title="This node holds the orchestrator seat. What you type in the command bar arrives here."
          >
            command center
          </span>
        )}
        {previewUrl !== null && (
          // The server's own address, read from its banner, and the one-click way to see the
          // page: clicking stands a browser node up next to this one. The offer is a click the
          // user takes, never a node that appears uninvited.
          <button
            className="identra-node__preview nodrag"
            title="The dev server is serving here. Click to open it in a browser node."
            onClick={() => nodeData.onOpenPreview?.(id, previewUrl)}
          >
            {previewUrl}
          </button>
        )}
        {nodeData.onToggleLock !== undefined && (
          // Always visible once locked, hover-only when open, the same as the close button. A lock
          // that hides itself is a setting the user cannot tell is on, and the whole value of this
          // is knowing at a glance which nodes agents cannot touch.
          <button
            className="identra-node__lock nodrag"
            data-on={nodeData.locked === true}
            title={
              nodeData.locked === true
                ? "Locked: agents cannot wire anything to this node. Click to unlock."
                : "Lock this node so agents cannot wire anything to it."
            }
            aria-pressed={nodeData.locked === true}
            onClick={() => nodeData.onToggleLock?.(id)}
          >
            {nodeData.locked === true ? "locked" : "lock"}
          </button>
        )}
        <button
          className="identra-node__close nodrag"
          title={`Close this ${nodeData.title || nodeData.kind}`}
          onClick={() => {
            // Naming what it is, because "delete this node" does not tell you that a conversation
            // goes with it. This is the only way to remove one now that the key does not.
            const name = nodeData.title || nodeData.kind;
            const cost = isDev
              ? "The dev server stops."
              : "The agent stops and its conversation is forgotten.";
            if (window.confirm(`Close ${name}?\n\n${cost}`)) {
              void deleteElements({ nodes: [{ id }] });
            }
          }}
        >
          &times;
        </button>
      </div>
      {recall.length > 0 && recallShown && (
        // Calm and earned, not a popup: it sits above the terminal, states what is known, and gets
        // out of the way the moment the human is done with it. This is the single most important
        // visual in the product, because it is the one that makes "it remembers" a thing you see
        // rather than a claim you read.
        <div className="identra-node__recall nodrag">
          <div className="identra-node__recall-head">
            <span>Identra remembers ({recall.length})</span>
            <button
              className="identra-node__recall-close"
              title="Hide what the project remembers"
              onClick={() => setRecallShown(false)}
            >
              &times;
            </button>
          </div>
          <ul>
            {recall.map((m) => (
              <li key={m.id}>{m.content}</li>
            ))}
          </ul>
        </div>
      )}
      <div className="identra-node__term nodrag nowheel" ref={termHost} />
    </div>
  );
}

export default memo(AgentNodeImpl);

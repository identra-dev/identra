// One pane of the centre column: the thing a leaf of the layout tree draws.
//
// This is the canvas node with the canvas taken out of it. What is gone is everything that existed
// because it was a box on a plane — the drag handles, the connection ports, the two sizes it had to
// remember, the overlay that opened a second copy at full window size. What is left is what a pane
// actually is: a header saying which node this is and what it is doing, and a body that is a
// terminal, a web view, a file, or a note.
//
// The process is not owned here. A pane can be closed and reopened, moved to the other half of a
// split, or replaced by a different node, and none of that touches the pty behind it. That is why
// there is no teardown call in here at all: closing a pane is a thing the user does to their view,
// and closing an agent is a thing they do to their agent, and the two used to be the same gesture
// only because a canvas had no way to tell them apart.
import { useCallback, useEffect, useRef, useState } from "react";
import { useAttachedTerminal } from "./attachTerminal";
import { fileRead, memoryList, type FileView, type Memory } from "./api";
import { appendTail, findLocalUrl } from "./devurl";
import { AgentIcon, auraFor } from "./icons";
import { useNodeState } from "./nodeState";
import { startNode } from "./spawn";
import type { CanvasNode } from "./api";

type Props = {
  node: CanvasNode;
  // Whether this is the pane the keyboard is talking to. Splits are only useful if you can tell
  // which half a keystroke lands in.
  focused: boolean;
  onFocus: () => void;
  // Close this pane. The node stays open in the tab bar and its process keeps running.
  onClosePane: () => void;
  // Is there more than one pane. With one, closing it would leave nowhere to put anything.
  closable: boolean;
  // A browser pane committing a new address, which is a change to the node and so a change to save.
  onSetCwd: (nodeId: string, cwd: string) => void;
  // A dev server announcing where it is serving, so the shell can offer to open it.
  onPreviewUrl: (nodeId: string, url: string) => void;
};

export default function Pane({
  node,
  focused,
  onFocus,
  onClosePane,
  closable,
  onSetCwd,
  onPreviewUrl,
}: Props) {
  const isTerminal =
    node.kind !== "browser" && node.kind !== "file" && node.kind !== "note";
  // Where a dev server says it is serving, once it has said so. It lives here rather than in the
  // body because the header is where it is drawn, and it is drawn as an offer the user takes: a
  // browser tab that opened by itself, uninvited, is the app doing something nobody asked for.
  const [previewUrl, setPreviewUrl] = useState<string | null>(null);
  return (
    <div
      className="identra-pane"
      data-focused={focused || undefined}
      // Focus follows the click rather than a keyboard focus ring, because the thing inside a pane
      // is usually a terminal that takes focus for itself the moment it is touched.
      onMouseDownCapture={onFocus}
      style={{ ["--aura" as string]: auraFor(node.kind) }}
    >
      <PaneHeader
        node={node}
        onClosePane={onClosePane}
        closable={closable}
        isTerminal={isTerminal}
        previewUrl={previewUrl}
        onOpenPreview={() => {
          if (previewUrl !== null) onPreviewUrl(node.id, previewUrl);
        }}
      />
      {isTerminal && <TerminalBody node={node} onFoundUrl={setPreviewUrl} />}
      {node.kind === "browser" && (
        <BrowserBody node={node} onSetCwd={onSetCwd} />
      )}
      {node.kind === "file" && <FileBody node={node} />}
      {node.kind === "note" && (
        <div className="identra-note__body">{node.title}</div>
      )}
    </div>
  );
}

function PaneHeader({
  node,
  onClosePane,
  closable,
  isTerminal,
  previewUrl,
  onOpenPreview,
}: {
  node: CanvasNode;
  onClosePane: () => void;
  closable: boolean;
  isTerminal: boolean;
  previewUrl: string | null;
  onOpenPreview: () => void;
}) {
  return (
    <div className="identra-pane__header">
      {isTerminal && <StateDot nodeId={node.id} />}
      <AgentIcon kind={node.kind} className="identra-node__icon" />
      <span className="identra-node__title">{node.title || node.kind}</span>
      {previewUrl !== null && (
        // The server's own address, read from its banner, and the one-click way to see the page.
        // The offer is a click the user takes, never a tab that appears uninvited.
        <button
          className="identra-node__preview"
          title="The dev server is serving here. Click to open it in a browser tab."
          onClick={onOpenPreview}
        >
          {previewUrl}
        </button>
      )}
      {closable && (
        <button
          className="identra-node__close"
          // Said out loud, because on a canvas this control killed the agent and here it does not.
          // Someone who learned the old meaning has to be told the new one at the moment they reach
          // for it, not after they have closed a pane expecting an agent to stop.
          title="Close this pane. The agent keeps running and stays in the tab bar."
          onClick={onClosePane}
        >
          &times;
        </button>
      )}
    </div>
  );
}

function StateDot({ nodeId }: { nodeId: string }) {
  const state = useNodeState(nodeId);
  const said = {
    ready: "Idle",
    running: "Working",
    "needs-input": "Waiting for an answer",
    exited: "Stopped",
  }[state];
  return <span className="identra-node__dot" data-state={state} title={said} />;
}

function TerminalBody({
  node,
  onFoundUrl,
}: {
  node: CanvasNode;
  onFoundUrl: (url: string) => void;
}) {
  const host = useRef<HTMLDivElement>(null);
  const isDev = node.kind === "dev";

  // What the project already knows, shown once when the pane opens. This is the payoff made
  // visible: the agent has not typed a word and the human can already see it is not starting cold.
  // A few facts, not the whole store, because it is a glance and not the memory panel.
  const [recall, setRecall] = useState<Memory[]>([]);
  const [recallShown, setRecallShown] = useState(true);
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

  // Stable across renders, because `useAttachedTerminal` rebuilds its terminal when they change and
  // a fresh closure every render would tear the pty's view down on every keystroke elsewhere.
  const start = useCallback(
    (rows: number, cols: number) =>
      startNode(node.id, node.kind, node.cwd, rows, cols),
    [node.id, node.kind, node.cwd],
  );

  // The preview address is fished out of the dev server's own banner. A rolling tail, because chunk
  // boundaries land anywhere, including mid-url.
  const scan = useRef({ tail: "", found: false, decoder: new TextDecoder() });
  const onBytes = useCallback(
    (bytes: Uint8Array) => {
      if (!isDev || scan.current.found) return;
      scan.current.tail = appendTail(
        scan.current.tail,
        scan.current.decoder.decode(bytes, { stream: true }),
      );
      const url = findLocalUrl(scan.current.tail);
      if (url !== null) {
        scan.current.found = true;
        onFoundUrl(url);
      }
    },
    [isDev, onFoundUrl],
  );

  useAttachedTerminal(host, node.id, {
    kind: node.kind,
    fontSize: 13,
    start,
    onBytes,
  });

  return (
    <>
      {recall.length > 0 && recallShown && (
        // Calm and earned, not a popup: it sits above the terminal, states what is known, and gets
        // out of the way the moment the human is done with it. This is the single most important
        // visual in the product, because it is the one that makes "it remembers" a thing you see
        // rather than a claim you read.
        <div className="identra-node__recall">
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
      <div className="identra-node__term" ref={host} />
    </>
  );
}

function BrowserBody({
  node,
  onSetCwd,
}: {
  node: CanvasNode;
  onSetCwd: (nodeId: string, cwd: string) => void;
}) {
  // `url` is what the iframe loads; `draft` is what is being typed. Splitting them keeps the frame
  // from reloading on every keystroke: it navigates only when a URL is committed.
  const [url, setUrl] = useState(node.cwd || "");
  const [draft, setDraft] = useState(url);
  const commit = (next: string) => {
    setUrl(next);
    onSetCwd(node.id, next);
  };
  return (
    <>
      <div className="identra-pane__url">
        <input
          value={draft}
          spellCheck={false}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") commit(e.currentTarget.value);
          }}
          onBlur={(e) => commit(e.currentTarget.value)}
        />
      </div>
      <iframe
        className="identra-node__frame"
        src={url}
        title={node.title || "browser"}
        // The page in here is whatever the user pointed at, and an unsandboxed iframe can navigate
        // the TOP window: one target="_top" link and the whole app is replaced by someone's
        // webpage, which is exactly what a tester hit on macOS. Scripts, same-origin and forms are
        // what a dev-server preview needs to run; walking out of the frame is the one thing this
        // list refuses.
        sandbox="allow-scripts allow-same-origin allow-forms"
      />
    </>
  );
}

type FileState =
  | { at: "loading" }
  | { at: "error"; why: string }
  | { at: "ready"; view: FileView; imageUrl: string | null };

function FileBody({ node }: { node: CanvasNode }) {
  const [state, setState] = useState<FileState>({ at: "loading" });
  useEffect(() => {
    let dropped = false;
    let url: string | null = null;
    fileRead(node.cwd ?? "").then(
      (view) => {
        if (dropped) return;
        if (view.kind === "image") {
          // A blob URL, revoked when the pane goes, so an image is bytes exactly once and never a
          // base64 string glued into the DOM.
          url = URL.createObjectURL(new Blob([new Uint8Array(view.bytes)]));
        }
        setState({ at: "ready", view, imageUrl: url });
      },
      (e) => {
        if (!dropped) setState({ at: "error", why: String(e) });
      },
    );
    return () => {
      dropped = true;
      if (url !== null) URL.revokeObjectURL(url);
    };
  }, [node.cwd]);

  return (
    <div className="identra-file">
      {state.at === "loading" && (
        <p className="identra-file__state">Reading...</p>
      )}
      {state.at === "error" && (
        <p className="identra-file__state" role="alert">
          {state.why}
        </p>
      )}
      {state.at === "ready" && state.view.kind === "text" && (
        <pre className="identra-file__text">{state.view.text}</pre>
      )}
      {state.at === "ready" && state.view.kind === "image" && (
        <img
          className="identra-file__image"
          src={state.imageUrl ?? ""}
          alt={state.view.name}
        />
      )}
      {state.at === "ready" && state.view.kind === "binary" && (
        <p className="identra-file__state">
          {state.view.name} is a binary file ({state.view.size} bytes), so there
          is nothing to read here.
        </p>
      )}
      {state.at === "ready" && state.view.kind === "toobig" && (
        <p className="identra-file__state">
          {state.view.name} is {Math.round(state.view.size / 1024)}KB, which is
          more than this viewer will load. Open it in your editor.
        </p>
      )}
    </div>
  );
}

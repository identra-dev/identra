import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  ReactFlow,
  Background,
  Controls,
  addEdge,
  applyEdgeChanges,
  applyNodeChanges,
  type Connection,
  type Edge as FEdge,
  type EdgeChange,
  type Node,
  type NodeChange,
  type Viewport,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import "@xterm/xterm/css/xterm.css";
import AgentNode, { type AgentNodeData } from "./AgentNode";
import BrowserNode from "./BrowserNode";
import NoteNode from "./NoteNode";
import Onboarding from "./Onboarding";
import WorkspacePicker from "./WorkspacePicker";
import WorkPanel from "./WorkPanel";
import WorkspaceMenu from "./WorkspaceMenu";
import { AgentIcon } from "./icons";
import {
  canvasCommandResult,
  canvasSave,
  detectAgents,
  isAdopted,
  noAgentsInstalled,
  onCanvasCommand,
  refreshAgents,
  terminalKill,
  workspaceOpen,
  workspaceOpenRecent,
  type AgentInfo,
  type CanvasCommand,
  type CanvasNode,
  type CanvasResult,
  type WorkspaceMeta,
} from "./api";

type FNode = Node<AgentNodeData>;

const nodeTypes = { agent: AgentNode, browser: BrowserNode, note: NoteNode };
// Long enough that a drag is one write rather than sixty, short enough that the window I have to
// flush on close stays small.
const SAVE_DEBOUNCE_MS = 400;
const DEFAULT_W = 480;
const DEFAULT_H = 320;

// Every agent kind renders through the one AgentNode; the kind rides in node data. A saved
// claude or gemini node must reload as itself, so the real kind flows both ways here.
function toFlow(n: CanvasNode): FNode {
  return {
    id: n.id,
    type: n.kind === "browser" || n.kind === "note" ? n.kind : "agent",
    position: { x: n.x, y: n.y },
    data: { title: n.title, cwd: n.cwd, kind: n.kind },
    style: { width: n.width || DEFAULT_W, height: n.height || DEFAULT_H },
  };
}

function toCanvasNode(n: FNode): CanvasNode {
  return {
    id: n.id,
    kind: n.data.kind,
    x: n.position.x,
    y: n.position.y,
    width: Number(n.style?.width) || DEFAULT_W,
    height: Number(n.style?.height) || DEFAULT_H,
    title: n.data.title || n.data.kind,
    cwd: n.data.cwd ?? null,
  };
}

export default function App() {
  const [workspace, setWorkspace] = useState<WorkspaceMeta | null>(null);
  const [nodes, setNodes] = useState<FNode[]>([]);
  const [edges, setEdges] = useState<FEdge[]>([]);
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  const [panelOpen, setPanelOpen] = useState(false);
  // Set when a write to disk fails. The board is on screen and not saved, and the only wrong move
  // is to say nothing.
  const [saveError, setSaveError] = useState<string | null>(null);
  const viewport = useRef<Viewport>({ x: 0, y: 0, zoom: 1 });
  // scheduleSave persists the whole canvas but each handler only has its own slice; these refs
  // hold the latest of both so a save always writes a consistent nodes+edges pair.
  const nodesRef = useRef<FNode[]>([]);
  const edgesRef = useRef<FEdge[]>([]);
  const titleRef = useRef("");
  const saveTimer = useRef<number | undefined>(undefined);
  // Is the board on screen different from the board on disk. This is what the close handler asks.
  const unsaved = useRef(false);
  // The canvas-command handler runs outside React's render, so it reads agents from a ref rather
  // than closing over state that would be stale by the time an agent calls.
  const agentsRef = useRef<AgentInfo[]>([]);

  useEffect(() => {
    void detectAgents().then((list) => {
      agentsRef.current = list;
      setAgents(list);
    });
  }, []);

  // The first-run panel offers a recheck so a user who just installed an agent does not have to
  // relaunch. This clears the probe cache and refreshes what both the dock and the panel read.
  const recheckAgents = useCallback(() => {
    void refreshAgents().then((list) => {
      agentsRef.current = list;
      setAgents(list);
    });
  }, []);

  // A terminal and an iframe both need the wheel for their own scrolling, so they swallow it, which
  // leaves you unable to zoom the canvas while the pointer is over a node. Holding the modifier
  // turns that off for as long as it is down: the class stops matching, and the wheel reaches the
  // canvas. Cheaper than hunting for empty space, and it is the same key you already hold to zoom.
  const [wheelToCanvas, setWheelToCanvas] = useState(false);
  useEffect(() => {
    const down = (e: KeyboardEvent) =>
      (e.metaKey || e.ctrlKey) && setWheelToCanvas(true);
    const up = (e: KeyboardEvent) =>
      !e.metaKey && !e.ctrlKey && setWheelToCanvas(false);
    // Releasing the key outside the window never fires keyup, which would leave it stuck on.
    const blur = () => setWheelToCanvas(false);
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
    window.addEventListener("blur", blur);
    return () => {
      window.removeEventListener("keydown", down);
      window.removeEventListener("keyup", up);
      window.removeEventListener("blur", blur);
    };
  }, []);

  // Opening is what makes a workspace active in the engine: it repoints the canvas, and writes the
  // bus config and the agent guide into that folder so any agent launched here can find its peers.
  const openWorkspace = useCallback(async (w: WorkspaceMeta) => {
    // Two lookups, because there are two kinds of id. A workspace Identra made is found by slug in
    // the root; a folder you opened is found by path on the remembered list. Both are chosen from a
    // list the engine built, which is what stops either from being a path the window made up.
    const canvas = isAdopted(w)
      ? await workspaceOpenRecent(w.path)
      : await workspaceOpen(w.slug);
    const loaded = canvas.nodes.map(toFlow);
    nodesRef.current = loaded;
    edgesRef.current = canvas.edges;
    titleRef.current = canvas.title;
    setNodes(loaded);
    setEdges(canvas.edges);
    viewport.current = canvas.viewport;
    setWorkspace(w);
  }, []);

  // The whole board, from the refs, so a save always writes a consistent nodes+edges pair.
  const snapshot = useCallback(
    () => ({
      nodes: nodesRef.current.map(toCanvasNode),
      edges: edgesRef.current.map((e) => ({
        id: e.id,
        source: e.source,
        target: e.target,
      })),
      viewport: viewport.current,
      title: titleRef.current,
    }),
    [],
  );

  // Write now and wait for it. A failure here is the user's layout not being on disk, so it goes on
  // the screen: this used to be a bare `void canvasSave(...)`, which meant a full disk or a
  // read-only workspace looked exactly like a successful save until the app was reopened and the
  // work was gone.
  const saveNow = useCallback(async () => {
    window.clearTimeout(saveTimer.current);
    try {
      await canvasSave(snapshot());
      unsaved.current = false;
      setSaveError(null);
    } catch (e) {
      // Leave unsaved set. The board on screen is still not the board on disk, and the next close
      // should try again rather than assume this one counted.
      setSaveError(String(e));
    }
  }, [snapshot]);

  // Debounced atomic save. The engine writes atomically; we just avoid thrashing on drag.
  const scheduleSave = useCallback(() => {
    unsaved.current = true;
    window.clearTimeout(saveTimer.current);
    saveTimer.current = window.setTimeout(() => {
      void saveNow();
    }, SAVE_DEBOUNCE_MS);
  }, [saveNow]);

  // Closing inside the debounce window drops whatever was moved last, and dragging a node and then
  // quitting is a completely ordinary thing to do. I hold the close, flush, then let it go.
  //
  // The question is "is there work not on disk", which is why it asks `unsaved` and not the timer:
  // clearTimeout does not reset the handle, so a timer ref is only ever undefined before the very
  // first save and would answer "yes, pending" forever after. If the flush fails the error is
  // already on screen, and I still close, because refusing to quit over a failed save traps someone
  // in an app they are trying to leave.
  useEffect(() => {
    const win = getCurrentWindow();
    const pending = win.onCloseRequested(async (event) => {
      if (!unsaved.current) return;
      event.preventDefault();
      await saveNow();
      void win.destroy();
    });
    return () => {
      void pending.then((unlisten) => unlisten());
    };
  }, [saveNow]);

  const onNodesChange = useCallback(
    (changes: NodeChange<FNode>[]) => {
      setNodes((cur) => {
        const next = applyNodeChanges(changes, cur);
        nodesRef.current = next;
        scheduleSave();
        return next;
      });
    },
    [scheduleSave],
  );

  const onEdgesChange = useCallback(
    (changes: EdgeChange<FEdge>[]) => {
      setEdges((cur) => {
        const next = applyEdgeChanges(changes, cur);
        edgesRef.current = next;
        scheduleSave();
        return next;
      });
    },
    [scheduleSave],
  );

  const onConnect = useCallback(
    (c: Connection) => {
      setEdges((cur) => {
        const next = addEdge(c, cur);
        edgesRef.current = next;
        scheduleSave();
        return next;
      });
    },
    [scheduleSave],
  );

  const onNodesDelete = useCallback((deleted: FNode[]) => {
    for (const n of deleted) void terminalKill(n.id);
  }, []);

  // Returns the new node's id, because an agent that asked for this needs to be able to name it.
  const addNode = useCallback(
    (
      kind: string,
      title: string,
      cwd: string | null = null,
      at?: { x: number; y: number },
    ) => {
      const vp = viewport.current;
      // Drop the node near the middle of what's currently on screen.
      const spot = at ?? {
        x: (-vp.x + window.innerWidth / 2 - DEFAULT_W / 2) / vp.zoom,
        y: (-vp.y + window.innerHeight / 2 - DEFAULT_H / 2) / vp.zoom,
      };
      const id = crypto.randomUUID();
      setNodes((cur) => {
        const next = [
          ...cur,
          toFlow({
            id,
            kind,
            x: spot.x,
            y: spot.y,
            width: DEFAULT_W,
            height: DEFAULT_H,
            title,
            cwd,
          }),
        ];
        nodesRef.current = next;
        scheduleSave();
        return next;
      });
      return id;
    },
    [scheduleSave],
  );

  const wire = useCallback(
    (from: string, to: string) => {
      setEdges((cur) => {
        const next = addEdge(
          { source: from, target: to, sourceHandle: null, targetHandle: null },
          cur,
        );
        edgesRef.current = next;
        scheduleSave();
        return next;
      });
    },
    [scheduleSave],
  );

  // An agent asking the canvas to change. The canvas is the single writer of its own state, so the
  // engine sends the request here rather than editing canvas.json underneath us, and we answer.
  // Every branch must reply exactly once: an agent is blocked on this until it hears back.
  const applyCanvasCommand = useCallback(
    (cmd: CanvasCommand): CanvasResult => {
      const p = cmd.params;
      switch (cmd.action) {
        case "add_terminal": {
          const kind = typeof p.kind === "string" ? p.kind : "codex";
          const known = agentsRef.current.find((a) => a.id === kind);
          // Refuse rather than drop a node that can never run. The agent gets a reason it can act
          // on, which is better than a broken node appearing on the user's canvas.
          if (!known)
            return {
              ok: false,
              error: `no agent called ${kind} is known here`,
            };
          if (!known.available) {
            return {
              ok: false,
              error: `${known.name} is not installed on this machine`,
            };
          }
          // Place a spawned node below its parent so a fan-out reads as a tree, not a pile.
          const parent = nodesRef.current.find((n) => n.id === p.connectTo);
          const at = parent
            ? { x: parent.position.x, y: parent.position.y + DEFAULT_H + 60 }
            : undefined;
          const title =
            typeof p.title === "string" && p.title ? p.title : known.name;
          const id = addNode(kind, title, null, at);
          if (typeof p.connectTo === "string" && p.connectTo)
            wire(p.connectTo, id);
          return { ok: true, id };
        }
        case "connect_nodes": {
          const { from, to } = p as { from?: string; to?: string };
          const has = (id?: string) =>
            nodesRef.current.some((n) => n.id === id);
          if (!has(from) || !has(to)) {
            return {
              ok: false,
              error: "one of those nodes is not on the canvas",
            };
          }
          if (from === to)
            return { ok: false, error: "a node cannot be wired to itself" };
          wire(from as string, to as string);
          return { ok: true, id: `${from}->${to}` };
        }
        case "add_note": {
          const text = typeof p.text === "string" ? p.text : "";
          if (!text.trim())
            return { ok: false, error: "a note needs some text" };
          return { ok: true, id: addNode("note", text) };
        }
        default:
          return {
            ok: false,
            error: `the canvas does not know how to ${cmd.action}`,
          };
      }
    },
    [addNode, wire],
  );

  useEffect(() => {
    const un = onCanvasCommand((cmd) => {
      let result: CanvasResult;
      try {
        result = applyCanvasCommand(cmd);
      } catch (e) {
        // Never leave the agent hanging on our bug: it waits on this reply.
        result = { ok: false, error: String(e) };
      }
      void canvasCommandResult(cmd.requestId, result);
    });
    return () => {
      void un.then((f) => f());
    };
  }, [applyCanvasCommand]);

  if (!workspace) {
    return <WorkspacePicker onOpen={(w) => void openWorkspace(w)} />;
  }

  return (
    <div className="identra-root">
      {saveError !== null && (
        // It stays until a save works. A canvas that is not on disk is not a thing to mention once
        // and then hide: every drag from here is work that will not be there tomorrow, and the user
        // is the only one who can do anything about a full disk or a folder they cannot write to.
        <div className="identra-save-error" role="alert">
          <strong>This workspace is not being saved.</strong> {saveError}
        </div>
      )}
      <ReactFlow<FNode>
        nodes={nodes}
        edges={edges}
        nodeTypes={nodeTypes}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        onConnect={onConnect}
        onNodesDelete={onNodesDelete}
        onMoveEnd={(_, vp) => {
          viewport.current = vp;
          scheduleSave();
        }}
        defaultViewport={viewport.current}
        nodesConnectable
        minZoom={0.2}
        maxZoom={2}
        // A node here is a running agent with a live conversation, not a shape. The default binds
        // Backspace and Delete to destroy the selection, which means one keystroke with a node
        // selected kills an agent mid-task and takes its session with it. Deleting goes through the
        // node menu, where it can ask first.
        deleteKeyCode={null}
        noWheelClassName={wheelToCanvas ? "identra-wheel-never" : "nowheel"}
        proOptions={{ hideAttribution: true }}
      >
        <Background color="#3a3a3a" gap={24} />
        <Controls showInteractive={false} />
      </ReactFlow>

      {/* A blank grid reads as a broken app. With no agent installed the dock is all disabled, so
          the usual hint would point at a dock you cannot use; show the install panel instead. Both
          go away the moment there is a node to look at. */}
      {nodes.length === 0 &&
        (noAgentsInstalled(agents) ? (
          <Onboarding agents={agents} onRecheck={recheckAgents} />
        ) : (
          <div className="identra-empty">
            <p className="identra-empty__lead">This workspace is empty.</p>
            <p className="identra-empty__hint">
              Pick an agent from the dock below to run it here. Drop in a second
              one and draw a wire between them, and they can split the work
              between themselves.
            </p>
          </div>
        ))}

      <div className="identra-topbar">
        <WorkspaceMenu
          workspace={workspace}
          onOpen={(w) => void openWorkspace(w)}
          onRenamed={setWorkspace}
          onDeleted={() => {
            // Back to the picker. The workspace under us is gone, so there is nothing to show and
            // nothing to save into.
            setWorkspace(null);
            setNodes([]);
            setEdges([]);
            nodesRef.current = [];
            edgesRef.current = [];
          }}
        />
        <button
          className="identra-topbar__btn"
          data-on={panelOpen}
          onClick={() => setPanelOpen((v) => !v)}
          title="What your agents are working on"
        >
          Work
        </button>
      </div>

      {panelOpen && <WorkPanel onClose={() => setPanelOpen(false)} />}

      <div className="identra-dock">
        {agents.map((a) => {
          const state = a.available
            ? a.logged_in
              ? "ready"
              : "setup"
            : "missing";
          return (
            <button
              key={a.id}
              className="identra-dock__btn"
              data-state={state}
              disabled={state === "missing"}
              title={
                a.available
                  ? a.logged_in
                    ? `${a.name}, signed in`
                    : `${a.name}, installed but not signed in`
                  : `${a.name}, not installed`
              }
              onClick={() => {
                // A signed-in agent just opens. One that is installed but not signed in would drop
                // the user into a raw login prompt with no idea why, so I name what is about to
                // happen first. The node still runs the real CLI, which is where the sign-in lives,
                // so this is a heads-up and not a second login path to keep in step.
                if (
                  state === "setup" &&
                  !window.confirm(
                    `${a.name} is installed but not signed in.\n\nOpening it will start its own sign-in in the node. Follow the prompts there, then the dot turns green.`,
                  )
                ) {
                  return;
                }
                addNode(a.id, a.name);
              }}
            >
              <AgentIcon kind={a.id} className="identra-dock__tile" />
              <span className="identra-dock__label">{a.name}</span>
              <span className="identra-dock__dot" data-state={state} />
            </button>
          );
        })}
        <button
          className="identra-dock__btn"
          data-state="ready"
          title="Browser, open a web view on the canvas"
          onClick={() => addNode("browser", "Browser", "http://localhost:1420")}
        >
          <AgentIcon kind="browser" className="identra-dock__tile" />
          <span className="identra-dock__label">Browser</span>
        </button>
      </div>
    </div>
  );
}

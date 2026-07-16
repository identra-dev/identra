import { useCallback, useEffect, useRef, useState } from "react";
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
import { iconFor } from "./icons";
import {
  canvasLoad,
  canvasSave,
  detectAgents,
  terminalKill,
  type AgentInfo,
  type Canvas,
  type CanvasNode,
} from "./api";

type FNode = Node<AgentNodeData>;

const nodeTypes = { agent: AgentNode, browser: BrowserNode };
const DEFAULT_W = 480;
const DEFAULT_H = 320;

// Every agent kind renders through the one AgentNode; the kind rides in node data. A saved
// claude or gemini node must reload as itself, so the real kind flows both ways here.
function toFlow(n: CanvasNode): FNode {
  return {
    id: n.id,
    type: n.kind === "browser" ? "browser" : "agent",
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
  const [nodes, setNodes] = useState<FNode[]>([]);
  const [edges, setEdges] = useState<FEdge[]>([]);
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  const viewport = useRef<Viewport>({ x: 0, y: 0, zoom: 1 });
  // scheduleSave persists the whole canvas but each handler only has its own slice; these refs
  // hold the latest of both so a save always writes a consistent nodes+edges pair.
  const nodesRef = useRef<FNode[]>([]);
  const edgesRef = useRef<FEdge[]>([]);
  const [ready, setReady] = useState(false);
  const saveTimer = useRef<number | undefined>(undefined);

  useEffect(() => {
    void canvasLoad().then((c: Canvas) => {
      const loaded = c.nodes.map(toFlow);
      nodesRef.current = loaded;
      edgesRef.current = c.edges; // old canvas.json with no edges deserializes to []
      setNodes(loaded);
      setEdges(c.edges);
      viewport.current = c.viewport;
      setReady(true);
    });
    void detectAgents().then(setAgents);
  }, []);

  // Debounced atomic save — the engine writes atomically; we just avoid thrashing on drag.
  const scheduleSave = useCallback(() => {
    window.clearTimeout(saveTimer.current);
    saveTimer.current = window.setTimeout(() => {
      void canvasSave({
        nodes: nodesRef.current.map(toCanvasNode),
        edges: edgesRef.current.map((e) => ({
          id: e.id,
          source: e.source,
          target: e.target,
        })),
        viewport: viewport.current,
      });
    }, 400);
  }, []);

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

  const addNode = useCallback(
    (kind: string, title: string, cwd: string | null = null) => {
      const vp = viewport.current;
      // Drop the node near the middle of what's currently on screen.
      const x = (-vp.x + window.innerWidth / 2 - DEFAULT_W / 2) / vp.zoom;
      const y = (-vp.y + window.innerHeight / 2 - DEFAULT_H / 2) / vp.zoom;
      setNodes((cur) => {
        const next = [
          ...cur,
          toFlow({
            id: crypto.randomUUID(),
            kind,
            x,
            y,
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
    },
    [scheduleSave],
  );

  if (!ready) return null;

  const browserIcon = iconFor("browser");

  return (
    <div className="identra-root">
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
        proOptions={{ hideAttribution: true }}
      >
        <Background color="#3a3a3a" gap={24} />
        <Controls showInteractive={false} />
      </ReactFlow>

      <div className="identra-dock">
        {agents.map((a) => {
          const icon = iconFor(a.id);
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
                    ? `${a.name} — signed in`
                    : `${a.name} — installed, not signed in`
                  : `${a.name} — not installed`
              }
              onClick={() => addNode(a.id, a.name)}
            >
              <span
                className="identra-dock__tile"
                style={{ background: icon.tile }}
              >
                {icon.glyph}
              </span>
              <span className="identra-dock__label">{a.name}</span>
              <span className="identra-dock__dot" data-state={state} />
            </button>
          );
        })}
        <button
          className="identra-dock__btn"
          data-state="ready"
          title="Browser — open a web view on the canvas"
          onClick={() => addNode("browser", "Browser", "http://localhost:5173")}
        >
          <span
            className="identra-dock__tile"
            style={{ background: browserIcon.tile }}
          >
            {browserIcon.glyph}
          </span>
          <span className="identra-dock__label">Browser</span>
        </button>
      </div>
    </div>
  );
}

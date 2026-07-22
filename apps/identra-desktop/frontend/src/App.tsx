import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  ReactFlow,
  Background,
  Controls,
  MiniMap,
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
import CommandBar, { type DispatchState } from "./CommandBar";
import WallpaperPicker from "./WallpaperPicker";
import { AgentIcon } from "./icons";
import { tidyPositions } from "./tidy";
import { backgroundCss, DEFAULT_WALLPAPER, dotColor, needsScrim } from "./wallpaper";
import {
  composeDispatch,
  planLine,
  planSeat,
  summarizePlan,
} from "./commandcenter";
import {
  boardList,
  canvasCommandResult,
  canvasExport,
  canvasImport,
  canvasSave,
  defaultOrchestrator,
  detectAgents,
  isAdopted,
  noAgentsInstalled,
  onCanvasCommand,
  refreshAgents,
  seatBrief,
  terminalInput,
  terminalKill,
  terminalStatus,
  workspaceOpen,
  workspaceOpenRecent,
  type AgentInfo,
  type CanvasCommand,
  type CanvasNode,
  type CanvasResult,
  type Wallpaper,
  type WorkspaceMeta,
} from "./api";

type FNode = Node<AgentNodeData>;

const nodeTypes = { agent: AgentNode, browser: BrowserNode, note: NoteNode };
// Long enough that a drag is one write rather than sixty, short enough that the window I have to
// flush on close stays small.
const SAVE_DEBOUNCE_MS = 400;
// How often the command bar re-reads the board and the seat's state. Slow enough to be free, fast
// enough that "it is asking you something" does not sit unnoticed. Only runs while a seat exists.
const SEAT_POLL_MS = 2500;
const DEFAULT_W = 480;
const DEFAULT_H = 320;

// Every agent kind renders through the one AgentNode; the kind rides in node data. A saved
// claude or gemini node must reload as itself, so the real kind flows both ways here.
function toFlow(n: CanvasNode): FNode {
  return {
    id: n.id,
    type: n.kind === "browser" || n.kind === "note" ? n.kind : "agent",
    position: { x: n.x, y: n.y },
    data: { title: n.title, cwd: n.cwd, kind: n.kind, locked: n.locked },
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
    locked: n.data.locked === true,
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
  // Which node holds the orchestrator seat. State because the canvas draws it, and a ref alongside
  // for the same reason the nodes have one: snapshot() runs outside render and has to write the
  // current seat, not the one from the render that scheduled the save.
  const [seat, setSeat] = useState<string | null>(null);
  const seatRef = useRef<string | null>(null);
  // The background this workspace wears. State because the canvas draws it, a ref so snapshot()
  // writes the current choice rather than the one from the render that scheduled the save.
  const [wallpaper, setWallpaper] = useState<Wallpaper>(DEFAULT_WALLPAPER);
  const wallpaperRef = useRef<Wallpaper>(DEFAULT_WALLPAPER);
  // Where the wallpaper popover is open, or null. Set by right-clicking the canvas background.
  const [wallMenu, setWallMenu] = useState<{ x: number; y: number } | null>(
    null,
  );
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
    // A seat pointing at a node that is no longer here reads as no seat. That happens whenever the
    // seat node was closed, and resolving it on load means nothing downstream has to keep asking
    // whether the seat still exists.
    const restored = canvas.nodes.some((n) => n.id === canvas.seat)
      ? canvas.seat
      : null;
    seatRef.current = restored;
    setSeat(restored);
    wallpaperRef.current = canvas.wallpaper;
    setWallpaper(canvas.wallpaper);
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
      seat: seatRef.current,
      wallpaper: wallpaperRef.current,
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

  // Straighten the board. Positions only: nothing is started, stopped, or rewired, so this is
  // always safe to press. It lays out into the top left of what is currently on screen rather than
  // at the canvas origin, because a canvas that has been panned would otherwise tidy itself out of
  // view and look like it had deleted everything.
  const [minimapOn, setMinimapOn] = useState(false);
  const tidy = useCallback(() => {
    const vp = viewport.current;
    const origin = { x: -vp.x / vp.zoom + 40, y: -vp.y / vp.zoom + 40 };
    const placed = new Map(
      tidyPositions(
        nodesRef.current.map((n) => ({
          id: n.id,
          position: n.position,
          width: Number(n.style?.width) || DEFAULT_W,
          height: Number(n.style?.height) || DEFAULT_H,
        })),
        origin,
      ).map((p) => [p.id, p]),
    );
    setNodes((cur) => {
      const next = cur.map((n) => {
        const at = placed.get(n.id);
        return at ? { ...n, position: { x: at.x, y: at.y } } : n;
      });
      nodesRef.current = next;
      scheduleSave();
      return next;
    });
  }, [scheduleSave]);

  // Take the board out to a file, or bring one in.
  //
  // Export sends what is on screen rather than what is on disk, so a change made in the last few
  // hundred milliseconds is in the file too. Both report through the save banner, which is already
  // the place this window says a canvas operation failed.
  const exportCanvas = useCallback(async () => {
    try {
      await canvasExport(snapshot());
    } catch (e) {
      setSaveError(`That canvas was not exported: ${String(e)}`);
    }
  }, [snapshot]);

  const importCanvas = useCallback(async () => {
    // Asked before the dialog opens, not after a file is chosen. Confirming a destructive action
    // and then being asked to pick the file is the wrong order: by then it reads as already decided.
    if (
      nodesRef.current.length > 0 &&
      !window.confirm(
        "Import a canvas?\n\nThis replaces the board in this workspace. The agents running here stop, and their conversations are forgotten.",
      )
    ) {
      return;
    }
    try {
      const imported = await canvasImport();
      if (imported === null) return; // cancelled, nothing to say
      // Stop what is running before the nodes go. These are the nodes being replaced, so the same
      // teardown a close does has to happen here or their PTYs outlive the board they belonged to.
      for (const n of nodesRef.current) {
        void terminalKill(n.id).catch(() => {
          // Best effort. The board is being replaced either way, and a node that would not die
          // cleanly is not a reason to leave the user looking at a canvas they just replaced.
        });
      }
      const loaded = imported.nodes.map(toFlow);
      nodesRef.current = loaded;
      edgesRef.current = imported.edges;
      titleRef.current = imported.title;
      const restored = imported.nodes.some((n) => n.id === imported.seat)
        ? imported.seat
        : null;
      seatRef.current = restored;
      setSeat(restored);
      // An imported board may reference an image that is not in this machine's library. It draws
      // as the plain background rather than erroring, which is the same fallback a removed
      // library file gets.
      wallpaperRef.current = imported.wallpaper;
      setWallpaper(imported.wallpaper);
      setNodes(loaded);
      setEdges(imported.edges);
      viewport.current = imported.viewport;
      // The engine already wrote it to disk as part of importing, so the window is in step with
      // the file rather than one debounce behind it.
      unsaved.current = false;
      setSaveError(null);
    } catch (e) {
      setSaveError(`That canvas was not imported: ${String(e)}`);
    }
  }, []);

  // Close a node to agents, or open it again. The user's own hands are never restricted by this:
  // they can still wire a locked node themselves, because it is their canvas and the lock is about
  // what happens while they are not watching.
  const toggleLock = useCallback(
    (nodeId: string) => {
      setNodes((cur) => {
        const next = cur.map((n) =>
          n.id === nodeId
            ? { ...n, data: { ...n.data, locked: n.data.locked !== true } }
            : n,
        );
        nodesRef.current = next;
        scheduleSave();
        return next;
      });
    },
    [scheduleSave],
  );

  // Picking a wallpaper applies immediately and rides the debounced save, exactly like moving a
  // node: the choice is one field on the canvas, not its own persistence path.
  const pickWallpaper = useCallback(
    (w: Wallpaper) => {
      wallpaperRef.current = w;
      setWallpaper(w);
      scheduleSave();
    },
    [scheduleSave],
  );

  // Moving the seat is one write. Nothing is spawned or killed here: the seat is a role, so taking
  // it from a node leaves that node running exactly as it was, just no longer the one the command
  // bar talks to.
  const assignSeat = useCallback(
    (nodeId: string | null) => {
      seatRef.current = nodeId;
      setSeat(nodeId);
      scheduleSave();
    },
    [scheduleSave],
  );

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

  // React Flow has already taken the node off the canvas by the time this runs, so this is where
  // the engine side goes: the PTY, the resumed conversation, and the node's bus credential. A node
  // that never launched has no terminal to kill and the engine says so; that is not a failure worth
  // showing anyone, but it must not become an unhandled rejection either.
  const onNodesDelete = useCallback(
    (deleted: FNode[]) => {
      for (const n of deleted) {
        void terminalKill(n.id).catch((err) => {
          console.warn(`could not close node ${n.id} cleanly`, err);
        });
      }
      // Closing the node that held the seat vacates it. The command bar then has nothing to talk
      // to and says so, which is better than dispatching into a node that is gone.
      if (deleted.some((n) => n.id === seatRef.current)) assignSeat(null);
    },
    [assignSeat],
  );

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
            locked: false,
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

  // The command center. One instruction goes to one node, and that node already holds every bus
  // tool it needs to break the work up and hand it out, so this adds no new mechanism: it is the
  // canvas typing into a terminal on the user's behalf.
  const [dispatch, setDispatch] = useState<DispatchState>({ kind: "idle" });
  // The seat is briefed once per session, in front of the first instruction it receives. Kept in a
  // ref rather than state because nothing renders from it and it must not be stale inside the async
  // dispatch below.
  const seatBriefed = useRef(false);

  // A freshly spawned node has no PTY for a moment: AgentNode starts the CLI after it mounts and
  // measures its terminal. Writing before that is writing into nothing, so I wait for the engine to
  // report a terminal under this id. I poll because the alternative is a started event that nothing
  // else needs, and the wait is short and only happens when the seat is being stood up.
  const waitForTerminal = useCallback(async (nodeId: string) => {
    const deadline = Date.now() + 15000;
    while (Date.now() < deadline) {
      // A null status means no terminal under that id yet, which is the state being waited out. A
      // rejection means the same thing from the other direction, so both just retry.
      const status = await terminalStatus(nodeId).catch(() => null);
      if (status !== null) return true;
      await new Promise((r) => setTimeout(r, 150));
    }
    return false;
  }, []);

  const sendToSeat = useCallback(
    async (instruction: string) => {
      const plan = planSeat(
        seatRef.current,
        nodesRef.current.map((n) => n.id),
        await defaultOrchestrator().catch(() => null),
      );

      if (plan.kind === "unavailable") {
        setDispatch({
          kind: "failed",
          error:
            "No installed agent can run the command center here. Install one of the supported agents, then try again.",
        });
        return;
      }

      let nodeId: string;
      let fresh = false;
      if (plan.kind === "use") {
        nodeId = plan.nodeId;
      } else {
        const agent = agentsRef.current.find((a) => a.id === plan.agentId);
        setDispatch({
          kind: "sending",
          note: `Starting ${agent?.name ?? plan.agentId} as the orchestrator`,
        });
        nodeId = addNode(plan.agentId, agent?.name ?? plan.agentId);
        assignSeat(nodeId);
        fresh = true;
        seatBriefed.current = false;
        if (!(await waitForTerminal(nodeId))) {
          setDispatch({
            kind: "failed",
            error: `${agent?.name ?? plan.agentId} did not start, so the instruction was not sent.`,
          });
          return;
        }
        // The CLI has a terminal but is still drawing its own first screen, and several of them
        // discard whatever is already pending when they take over the tty. A short settle costs one
        // beat on the first instruction of a session and saves silently losing it.
        await new Promise((r) => setTimeout(r, 1200));
      }

      setDispatch({ kind: "sending", note: "Sending to the orchestrator" });
      try {
        await terminalInput(
          nodeId,
          composeDispatch(
            await seatBrief(),
            instruction,
            fresh || !seatBriefed.current,
          ),
        );
        seatBriefed.current = true;
        setDispatch({
          kind: "sent",
          note: "Sent. Watch the orchestrator node for what it does next.",
        });
      } catch (e) {
        // The seat node was closed, or its agent has exited. Either way the instruction did not
        // land, and the user is the only one who can do anything about it.
        setDispatch({
          kind: "failed",
          error: `That did not reach the orchestrator: ${String(e)}`,
        });
      }
    },
    [addNode, assignSeat, waitForTerminal],
  );

  // What the seat is doing, shown next to the bar so the user does not have to read a scrolling
  // terminal to know whether anything came of what they typed.
  const [plan, setPlan] = useState<string | null>(null);
  const [seatAsking, setSeatAsking] = useState(false);

  // Polled rather than pushed, and only while a seat exists. The board is written by agents through
  // the bus and the seat's status is read from output timing, so neither has an event to subscribe
  // to. Two cheap reads every few seconds is the honest cost of showing this at all, and it stops
  // entirely when there is no seat.
  useEffect(() => {
    if (seat === null) {
      setPlan(null);
      setSeatAsking(false);
      return;
    }
    let dropped = false;
    const poll = async () => {
      // Both are best effort. The board can be mid-write and the seat can be closed between the
      // check and the call, and neither is worth a visible error: the strip just keeps its last
      // reading until the next tick.
      const tasks = await boardList().catch(() => null);
      const status = await terminalStatus(seat).catch(() => null);
      if (dropped) return;
      if (tasks !== null) setPlan(planLine(summarizePlan(tasks)));
      setSeatAsking(status === "needs-input");
    };
    void poll();
    const timer = window.setInterval(() => void poll(), SEAT_POLL_MS);
    return () => {
      dropped = true;
      window.clearInterval(timer);
    };
  }, [seat]);

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
  //
  // This is also the one place a node's lock is enforced, and it is the right place: it is the only
  // door an agent has onto the canvas. The user's own drags go through onConnect and are never
  // checked, which is the intended asymmetry.
  const applyCanvasCommand = useCallback(
    (cmd: CanvasCommand): CanvasResult => {
      const p = cmd.params;
      const locked = (id?: string) =>
        nodesRef.current.some((n) => n.id === id && n.data.locked === true);
      // Named, so the agent can tell the user which node it was and they can decide, rather than
      // just reporting that something was refused.
      const lockedReason = (id: string) => {
        const name =
          nodesRef.current.find((n) => n.id === id)?.data.title ?? id;
        return `${name} is locked, so it cannot be wired to by an agent. The person at the keyboard can unlock it or wire it themselves.`;
      };
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
          // Refuse before spawning, not after. Creating the node and then failing to wire it would
          // leave a stray agent running on the user's canvas that nobody asked for and nobody owns.
          if (
            typeof p.connectTo === "string" &&
            p.connectTo &&
            locked(p.connectTo)
          ) {
            return { ok: false, error: lockedReason(p.connectTo) };
          }
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
          // Checking for undefined here as well as membership is what narrows both to a string for
          // the rest of the branch, so the wire call below needs no cast to say what it already
          // knows.
          if (
            from === undefined ||
            to === undefined ||
            !has(from) ||
            !has(to)
          ) {
            return {
              ok: false,
              error: "one of those nodes is not on the canvas",
            };
          }
          if (from === to)
            return { ok: false, error: "a node cannot be wired to itself" };
          // Either end being locked is enough to refuse. An edge is the bus authorization and it
          // reads both ways, so wiring out of a locked node exposes it exactly as much as wiring in.
          if (locked(from)) return { ok: false, error: lockedReason(from) };
          if (locked(to)) return { ok: false, error: lockedReason(to) };
          wire(from, to);
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

  // The seat is canvas state, not node state, so it is stamped onto the nodes at render rather than
  // stored in them. That keeps one seat id as the only truth, and it keeps `seat` out of what gets
  // written back to canvas.json as part of a node.
  const flowNodes = useMemo(
    () =>
      nodes.map((n) => {
        const data = { ...n.data, onToggleLock: toggleLock };
        return n.id === seat
          ? { ...n, data: { ...data, seat: true }, className: "is-seat" }
          : { ...n, data };
      }),
    [nodes, seat, toggleLock],
  );

  const seatName =
    seat === null
      ? null
      : (nodes.find((n) => n.id === seat)?.data.title ?? null);

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
      {/* The wallpaper is a layer behind the flow, not the flow's own background, so the grid
          dots and the nodes always sit above it. data-scrim pulls a user image toward the app
          background; the built-ins and swatches are curated dark values and need no help. */}
      <div
        className="identra-wallpaper"
        data-scrim={needsScrim(wallpaper) || undefined}
        style={{ background: backgroundCss(wallpaper, convertFileSrc) }}
      />
      <ReactFlow<FNode>
        nodes={flowNodes}
        edges={edges}
        nodeTypes={nodeTypes}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        onConnect={onConnect}
        onNodesDelete={onNodesDelete}
        onPaneContextMenu={(e) => {
          // Right-clicking the empty canvas is where you change what the empty canvas looks
          // like. The browser menu would cover ours, so it goes.
          e.preventDefault();
          setWallMenu({ x: e.clientX, y: e.clientY });
        }}
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
        {/* The dots flip to white over anything with its own character, because grey dots
            disappear into a picture. Over the plain board they keep their quiet grey. */}
        <Background color={dotColor(wallpaper)} gap={24} />
        <Controls showInteractive={false} />
        {/* Off by default. On a canvas with three nodes it is a box covering the corner for no
            gain; it earns its place once the command center has spawned enough helpers that the
            board runs off screen, which is exactly when the user goes looking for it. */}
        {minimapOn && (
          <MiniMap
            pannable
            zoomable
            className="identra-minimap"
            maskColor="rgba(20, 20, 20, 0.6)"
            nodeColor="#5e5c64"
          />
        )}
      </ReactFlow>

      {wallMenu !== null && (
        <WallpaperPicker
          current={wallpaper}
          at={wallMenu}
          onPick={pickWallpaper}
          onClose={() => setWallMenu(null)}
        />
      )}

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
        {/* Both are hidden on an empty canvas. There is nothing to tidy and nothing to map, and a
            row of controls that do nothing is how an empty state stops reading as a first run. */}
        {nodes.length > 0 && (
          <>
            <button
              className="identra-topbar__btn"
              onClick={tidy}
              title="Lay the nodes out on a grid. Moves them, nothing else."
            >
              Tidy
            </button>
            <button
              className="identra-topbar__btn"
              data-on={minimapOn}
              onClick={() => setMinimapOn((v) => !v)}
              title="Show a map of the whole canvas"
            >
              Map
            </button>
            <button
              className="identra-topbar__btn"
              onClick={() => void exportCanvas()}
              title="Save this canvas to a file"
            >
              Export
            </button>
          </>
        )}
        {/* Import stays available on an empty canvas: bringing a board in is exactly what you want
            to do with an empty one, and it is the only one of these that is useful with nothing
            on screen. */}
        <button
          className="identra-topbar__btn"
          onClick={() => void importCanvas()}
          title="Replace this canvas with one from a file"
        >
          Import
        </button>
      </div>

      {panelOpen && <WorkPanel onClose={() => setPanelOpen(false)} />}

      {/* Above the dock, because the dock is how you place one agent yourself and this is how you
          ask for the whole job to be done. Hidden until an agent exists to run it: on a machine
          with nothing installed the onboarding panel is the thing to read, and a command bar that
          can only fail is worse than no command bar. */}
      {!noAgentsInstalled(agents) && (
        <CommandBar
          seatName={seatName}
          state={dispatch}
          plan={plan}
          awaitingAnswer={seatAsking}
          onSubmit={(instruction) => void sendToSeat(instruction)}
        />
      )}

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

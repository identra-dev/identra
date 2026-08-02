import { useCallback, useEffect, useRef, useState } from "react";
import logo from "./assets/identra.png";
import { convertFileSrc } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "@xterm/xterm/css/xterm.css";
import FilesPanel from "./FilesPanel";
import Onboarding from "./Onboarding";
import Pane from "./Pane";
import WorkspacePicker from "./WorkspacePicker";
import SettingsPanel from "./SettingsPanel";
import WorkPanel from "./WorkPanel";
import WorkspaceMenu from "./WorkspaceMenu";
import CommandBar, { MOD_LABEL, type DispatchState } from "./CommandBar";
import ConnectionsPanel from "./ConnectionsPanel";
import WallpaperPicker from "./WallpaperPicker";
import { AgentIcon } from "./icons";
import {
  clearNode,
  closeLeaf,
  leaf,
  leaves,
  setNode,
  splitLeaf,
  stepLeaf,
  type Pane as PaneTree,
} from "./layout";
import { useNodeState } from "./nodeState";
import { backgroundCss, DEFAULT_WALLPAPER, needsScrim } from "./wallpaper";
import {
  composeDispatch,
  planLine,
  planSeat,
  summarizePlan,
} from "./commandcenter";
import { ago } from "./ago";
import {
  agentsByKind,
  boardList,
  busHandshakes,
  canvasCommandResult,
  canvasExport,
  canvasImport,
  canvasSave,
  defaultOrchestrator,
  devCommand,
  detectAgents,
  isAdopted,
  memoryList,
  memoryRevealOnce,
  noAgentsInstalled,
  onCanvasCommand,
  refreshAgents,
  seatBrief,
  terminalSend,
  terminalKill,
  terminalStart,
  terminalStatus,
  workspaceOpen,
  workspaceOpenRecent,
  type AgentInfo,
  type CanvasCommand,
  type CanvasNode,
  type CanvasResult,
  type Edge,
  type Handshake,
  type Grantor,
  type Viewport,
  type Wallpaper,
  type WorkspaceMeta,
} from "./api";

// Long enough that a burst of changes is one write rather than sixty, short enough that the window
// I have to flush on close stays small.
const SAVE_DEBOUNCE_MS = 400;
// The longest the window waits for its final save before closing anyway. Long enough that an
// ordinary write to a local file finishes inside it many times over, short enough that a user who
// clicked close is not left wondering whether the click registered.
const CLOSE_FLUSH_MAX_MS = 2000;
// How often the command bar re-reads the board and the seat's state. Slow enough to be free, fast
// enough that "it is asking you something" does not sit unnoticed. Only runs while a seat exists.
const SEAT_POLL_MS = 2500;
// How often the shell re-reads how many facts the project has learned, for the badge and the
// one-time reveal. Matches the panel's own poll: two small reads a few seconds apart cost nothing.
const MEMORY_POLL_MS = 2000;
// The headless orchestrator still runs inside a PTY, and a PTY has a size whether or not anyone is
// looking at it. This is only the size it is born at: the moment the command center pane mounts it
// re-sizes the PTY to the box actually showing it, the same as any pane. What these have to be is
// big enough that the CLI's first screen, drawn before the pane has attached, is not folded into
// nonsense that then has to be re-wrapped.
const SEAT_COLS = 120;
const SEAT_ROWS = 40;
// What a node's saved box used to be. Nothing draws at this size any more — a pane is whatever the
// split tree gives it — but the fields are still in the file, and writing a plausible number keeps
// a canvas exported from here readable by anything that still reads them.
const DEFAULT_W = 480;
const DEFAULT_H = 320;

// The right column's modes. Files and the work panel already existed as slide-overs; docking them
// is the change. Connections is new, and it is not a convenience: it is the only place a grant of
// agent-to-agent access can now be seen. Changes and Review are named in the plan and are not
// built, and an empty tab that says "coming soon" is worse than a column with three honest ones.
type RightMode = "work" | "files" | "connections";

// Whether this workspace has been told its canvas is gone. Kept in the browser's own storage rather
// than in the engine, because it is a fact about what this person has read and not about the
// project: no agent needs it, nothing else reads it, and putting it in canvas.json would mean a
// migration for a sentence.
//
// ponytail: localStorage, per workspace. If the notice ever has to survive a reinstall, it moves to
// the same place `memory_reveal_once` lives.
const CANVAS_NOTICE_KEY = "identra:canvas-gone:";

export default function App() {
  const [workspace, setWorkspace] = useState<WorkspaceMeta | null>(null);
  const [nodes, setNodes] = useState<CanvasNode[]>([]);
  const [edges, setEdges] = useState<Edge[]>([]);
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  // Which mode the right column is showing, or null when it is collapsed.
  const [right, setRight] = useState<RightMode | null>(null);
  // How many facts this project has learned. Drives the ambient badge and the one-time reveal, and
  // is polled whether or not the column is open, so the badge is right even while it is closed.
  const [memoryCount, setMemoryCount] = useState(0);
  const [settingsOpen, setSettingsOpen] = useState(false);
  // The dev command this workspace declares, or null. Existence is what the Run button keys on.
  const [devCmd, setDevCmd] = useState<string[] | null>(null);
  // Set when a write to disk fails. The work is on screen and not saved, and the only wrong move is
  // to say nothing.
  const [saveError, setSaveError] = useState<string | null>(null);
  // Said once per workspace, on the first open after the canvas went away.
  const [canvasNotice, setCanvasNotice] = useState(false);
  // What each agent was handed when it connected, by node id.
  //
  // This is the whole of the product's claim made visible. An agent opening already holding the
  // project is the one thing here a competitor cannot match by writing a better tool description,
  // and it happens entirely out of sight: the handshake is between that agent's CLI and the bus.
  // The canvas at least gave a person something to watch; with it gone, this line is the only
  // evidence the mechanism fired at all.
  const [handshakes, setHandshakes] = useState<Record<string, Handshake>>({});

  // The centre column. Session-only by design: see the note at the top of layout.ts.
  const [tree, setTree] = useState<PaneTree>(() => leaf("pane-0"));
  const [focusLeaf, setFocusLeaf] = useState("pane-0");
  const treeRef = useRef<PaneTree>(tree);
  treeRef.current = tree;
  const focusLeafRef = useRef(focusLeaf);
  focusLeafRef.current = focusLeaf;

  // scheduleSave persists the whole workspace but each handler only has its own slice; these refs
  // hold the latest of both so a save always writes a consistent nodes+edges pair.
  const nodesRef = useRef<CanvasNode[]>([]);
  const edgesRef = useRef<Edge[]>([]);
  const titleRef = useRef("");
  // The viewport is dead as a concept and alive as a field: nothing pans or zooms any more, but
  // canvas.json still carries one and a workspace last saved by v0.1.2 has a real value in it.
  // Round-tripping what was read keeps this window from being the thing that rewrote it.
  const viewportRef = useRef<Viewport>({ x: 0, y: 0, zoom: 1 });
  // Which node holds the orchestrator seat. State because the bar draws it, and a ref alongside for
  // the same reason the nodes have one: snapshot() runs outside render and has to write the current
  // seat, not the one from the render that scheduled the save.
  const [seat, setSeat] = useState<string | null>(null);
  // The display name of whatever agent is holding the seat, for the bar's label.
  const [seatAgent, setSeatAgent] = useState<string | null>(null);
  const seatRef = useRef<string | null>(null);
  // The background this workspace wears. It is behind the columns now rather than under nodes, so
  // it shows at the edges and through the gaps; the field and the picker are unchanged.
  const [wallpaper, setWallpaper] = useState<Wallpaper>(DEFAULT_WALLPAPER);
  const wallpaperRef = useRef<Wallpaper>(DEFAULT_WALLPAPER);
  const [wallMenu, setWallMenu] = useState<{ x: number; y: number } | null>(
    null,
  );
  const saveTimer = useRef<number | undefined>(undefined);
  // Is what is on screen different from what is on disk. This is what the close handler asks.
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
  // relaunch. This clears the probe cache and refreshes what both the sidebar and the panel read.
  // It returns the promise so the panel can show a checking state and a failure, rather than a
  // button that eats the click in silence.
  const recheckAgents = useCallback(async () => {
    const list = await refreshAgents();
    agentsRef.current = list;
    setAgents(list);
  }, []);

  // Backspace outside a text field is history-back in WebKit, and the shell's history is the app
  // itself: one stray keypress with nothing focused and the window walks backward out of Identra.
  // Editable targets keep the key, which covers every input here including xterm's hidden textarea,
  // so typing is untouched and only the navigation gesture dies.
  useEffect(() => {
    const guard = (e: KeyboardEvent) => {
      if (e.key !== "Backspace") return;
      const t = e.target;
      const editable =
        t instanceof HTMLInputElement ||
        t instanceof HTMLTextAreaElement ||
        (t instanceof HTMLElement && t.isContentEditable);
      if (!editable) e.preventDefault();
    };
    window.addEventListener("keydown", guard);
    return () => window.removeEventListener("keydown", guard);
  }, []);

  // Opening is what makes a workspace active in the engine: it repoints the window, and writes the
  // bus config and the agent guide into that folder so any agent launched here can find its peers.
  const openWorkspace = useCallback(async (w: WorkspaceMeta) => {
    // Two lookups, because there are two kinds of id. A workspace Identra made is found by slug in
    // the root; a folder you opened is found by path on the remembered list. Both are chosen from a
    // list the engine built, which is what stops either from being a path the window made up.
    const canvas = isAdopted(w)
      ? await workspaceOpenRecent(w.path)
      : await workspaceOpen(w.slug);
    nodesRef.current = canvas.nodes;
    edgesRef.current = canvas.edges;
    setEdges(canvas.edges);
    titleRef.current = canvas.title;
    viewportRef.current = canvas.viewport;
    // Opening a workspace always starts with no seat. A seat is a running process, and processes do
    // not survive the app closing, so a restored id could only ever name a PTY that is gone — and
    // the first instruction stands a fresh one up anyway.
    seatRef.current = null;
    setSeat(null);
    setSeatAgent(null);
    wallpaperRef.current = canvas.wallpaper;
    setWallpaper(canvas.wallpaper);
    setNodes(canvas.nodes);
    // One pane, showing whatever was made first. Nodes become tabs in the order they were created,
    // which is the only ordering the file still carries now that positions are not read.
    const first = canvas.nodes[0]?.id ?? null;
    setTree(leaf("pane-0", first));
    setFocusLeaf("pane-0");
    setWorkspace(w);
    // The one thing this window knows and the user does not: their arrangement is gone on purpose.
    // A canvas with everything at the origin was never arranged, so it gets no notice.
    const arranged = canvas.nodes.some((n) => n.x !== 0 || n.y !== 0);
    const key = CANVAS_NOTICE_KEY + w.slug;
    setCanvasNotice(arranged && window.localStorage.getItem(key) === null);
    // Whether this project declares a dev command decides whether the Run control exists at all.
    // Probed per open, because it is a property of the folder, not of the app.
    setDevCmd(null);
    void devCommand().then(setDevCmd, () => setDevCmd(null));
  }, []);

  const dismissCanvasNotice = useCallback(() => {
    setCanvasNotice(false);
    if (workspace !== null) {
      window.localStorage.setItem(CANVAS_NOTICE_KEY + workspace.slug, "read");
    }
  }, [workspace]);

  // The whole workspace, from the refs, so a save always writes a consistent nodes+edges pair.
  const snapshot = useCallback(
    () => ({
      nodes: nodesRef.current,
      edges: edgesRef.current,
      viewport: viewportRef.current,
      title: titleRef.current,
      seat: seatRef.current,
      wallpaper: wallpaperRef.current,
    }),
    [],
  );

  // Write now and wait for it. A failure here is the user's work not being on disk, so it goes on
  // the screen: a bare `void canvasSave(...)` would make a full disk or a read-only workspace look
  // exactly like a successful save until the app was reopened and the work was gone.
  const saveNow = useCallback(async () => {
    window.clearTimeout(saveTimer.current);
    try {
      await canvasSave(snapshot());
      unsaved.current = false;
      setSaveError(null);
    } catch (e) {
      // Leave unsaved set. What is on screen is still not what is on disk, and the next close
      // should try again rather than assume this one counted.
      setSaveError(String(e));
    }
  }, [snapshot]);

  // Debounced atomic save. The engine writes atomically; we just avoid thrashing.
  const scheduleSave = useCallback(() => {
    unsaved.current = true;
    window.clearTimeout(saveTimer.current);
    saveTimer.current = window.setTimeout(() => {
      void saveNow();
    }, SAVE_DEBOUNCE_MS);
  }, [saveNow]);

  // Back to the picker.
  //
  // Everything this clears is deliberate. The nodes and edges go because the next thing rendered is
  // the picker and stale ones would flash up under whatever is opened next. The seat goes because it
  // names a PTY belonging to the workspace being left. What does not happen here is killing the
  // agents: they are this workspace's processes and they keep running, so coming back finds them
  // where you left them.
  const goHome = useCallback(async () => {
    // What is on screen may be newer than what is on disk. Leaving is exactly the moment that
    // matters, and unlike closing there is no bound needed: nothing is waiting on it.
    await saveNow().catch(() => {});
    setWorkspace(null);
    setNodes([]);
    setEdges([]);
    nodesRef.current = [];
    edgesRef.current = [];
    seatRef.current = null;
    setSeat(null);
    setSeatAgent(null);
    setRight(null);
    setSettingsOpen(false);
    setTree(leaf("pane-0"));
    setFocusLeaf("pane-0");
  }, [saveNow]);

  // What to say when the window refuses to go. Shares the save banner because it is the same kind
  // of message — something the app cannot fix, that the user is the only one who can act on, and
  // that must stay on screen rather than being mentioned once.
  const reportStuck = useCallback((e: unknown) => {
    setSaveError(
      `Identra could not close itself: ${String(e)}. Close the window from your desktop instead.`,
    );
  }, []);

  // Closing inside the debounce window drops whatever changed last, and making a change and then
  // quitting is a completely ordinary thing to do. I hold the close, flush, then let it go.
  //
  // One close request owns the exit. A user who clicks close twice while the flush runs must not
  // start a second save or, worse, race two destroys; and whatever the save does, the window has to
  // actually go, because an app that refuses to close is holding its user hostage over a write they
  // cannot see. A tester on macOS hit exactly that wedge.
  const closing = useRef(false);
  useEffect(() => {
    const win = getCurrentWindow();
    const pending = win.onCloseRequested(async (event) => {
      // Asked twice. The first request is still flushing, and the honest reading of a second click
      // is "I want out now", so it goes now. This used to preventDefault and return, which was fine
      // exactly as long as the first request always reached its destroy. When it did not, the flag
      // stayed latched and every close from then on was refused.
      if (closing.current) {
        void win.destroy().catch(reportStuck);
        return;
      }
      if (!unsaved.current) return;
      event.preventDefault();
      closing.current = true;
      // Bounded, and that bound is the point. `canvas_save` crosses IPC into the engine, and an
      // engine that is wedged must not take the window with it: flushing on close exists to save
      // the user's work, not to make quitting conditional on a write succeeding.
      await Promise.race([
        saveNow().catch(() => {}),
        new Promise((resolve) => setTimeout(resolve, CLOSE_FLUSH_MAX_MS)),
      ]);
      void win.destroy().catch(reportStuck);
    });
    return () => {
      void pending.then((unlisten) => unlisten());
    };
  }, [saveNow, reportStuck]);

  // Ask the window to close, and say so out loud if it will not.
  //
  // The `.catch` is the whole reason this bug survived several rounds of being fixed. Window close
  // and destroy are permissioned in Tauri, `core:window:default` grants neither, and every call
  // site here was `void win.close()` with nothing watching. So the rejection went nowhere: the
  // button did nothing, alt+F4 did nothing, and there was no error anywhere to say why.
  const closeIdentra = useCallback(async () => {
    try {
      await getCurrentWindow().close();
    } catch (e) {
      reportStuck(e);
    }
  }, [reportStuck]);

  // Nodes are a list now, so every change to one is a list operation and they all save the same way.
  const putNodes = useCallback(
    (next: CanvasNode[]) => {
      nodesRef.current = next;
      setNodes(next);
      scheduleSave();
    },
    [scheduleSave],
  );

  const putEdges = useCallback(
    (next: Edge[]) => {
      edgesRef.current = next;
      setEdges(next);
      scheduleSave();
    },
    [scheduleSave],
  );

  // Returns the new node's id, because an agent that asked for this needs to be able to name it.
  // The new node also takes the focused pane, which is what makes opening an agent feel like opening
  // a thing rather than adding a row to a list you then have to click.
  const addNode = useCallback(
    (kind: string, title: string, cwd: string | null = null) => {
      const id = crypto.randomUUID();
      putNodes([
        ...nodesRef.current,
        {
          id,
          kind,
          x: 0,
          y: 0,
          width: DEFAULT_W,
          height: DEFAULT_H,
          title,
          cwd,
          locked: false,
        },
      ]);
      setTree((cur) => setNode(cur, focusLeafRef.current, id));
      return id;
    },
    [putNodes],
  );

  // Close a node for good: the process, the conversation, the tab, and any pane showing it. This is
  // the gesture the canvas spelled as deleting a box, and it is the only one that ends an agent.
  const closeNode = useCallback(
    (id: string) => {
      const node = nodesRef.current.find((n) => n.id === id);
      const name = node?.title || node?.kind || "this";
      const cost =
        node?.kind === "dev"
          ? "The dev server stops."
          : "The agent stops and its conversation is forgotten.";
      if (!window.confirm(`Close ${name}?\n\n${cost}`)) return;
      void terminalKill(id).catch((err) => {
        // A node that never launched has no terminal to kill and the engine says so. That is not a
        // failure worth showing anyone, but it must not become an unhandled rejection either.
        console.warn(`could not close node ${id} cleanly`, err);
      });
      putNodes(nodesRef.current.filter((n) => n.id !== id));
      // An edge whose end is gone is not a permission any more, it is a dangling id. The bus reads
      // this slice per call, so leaving it would be a grant pointing at nothing.
      putEdges(
        edgesRef.current.filter((e) => e.source !== id && e.target !== id),
      );
      setTree((cur) => clearNode(cur, id));
    },
    [putNodes, putEdges],
  );

  const setNodeCwd = useCallback(
    (id: string, cwd: string) => {
      putNodes(nodesRef.current.map((n) => (n.id === id ? { ...n, cwd } : n)));
    },
    [putNodes],
  );

  // Connect two nodes, recording who asked. `by` is not bookkeeping: an agent can call
  // `connect_nodes` and grant itself access to another agent's context, and the connections panel is
  // now the only place that shows up. A grant with no attribution is one the user cannot audit.
  const wire = useCallback(
    (from: string, to: string, by: Grantor) => {
      // Same pair twice is the same permission, and a second edge would mean revoking took two
      // clicks to do one thing. The first grant keeps its attribution: an agent re-asking for a
      // connection you already made does not make it the agent's.
      if (edgesRef.current.some((e) => e.source === from && e.target === to)) {
        return;
      }
      putEdges([
        ...edgesRef.current,
        { id: `${from}->${to}`, source: from, target: to, by },
      ]);
    },
    [putEdges],
  );

  // Take a connection back. `get_peer_context` reads this slice on every call, so a revoke takes
  // effect on the next read rather than at the next launch — unlike granting one, which a CLI only
  // picks up when it next starts. The asymmetry is the right way round: a permission should be
  // slower to give than to take away.
  const revoke = useCallback(
    (edgeId: string) => {
      putEdges(edgesRef.current.filter((e) => e.id !== edgeId));
    },
    [putEdges],
  );

  // Take the workspace out to a file, or bring one in.
  //
  // Export sends what is on screen rather than what is on disk, so a change made in the last few
  // hundred milliseconds is in the file too. Both report through the save banner, which is already
  // the place this window says a workspace operation failed.
  const exportCanvas = useCallback(async () => {
    try {
      await canvasExport(snapshot());
    } catch (e) {
      setSaveError(`That workspace was not exported: ${String(e)}`);
    }
  }, [snapshot]);

  const importCanvas = useCallback(async () => {
    // Asked before the dialog opens, not after a file is chosen. Confirming a destructive action
    // and then being asked to pick the file is the wrong order: by then it reads as already decided.
    if (
      nodesRef.current.length > 0 &&
      !window.confirm(
        "Import a workspace?\n\nThis replaces everything open here. The agents running here stop, and their conversations are forgotten.",
      )
    ) {
      return;
    }
    try {
      const imported = await canvasImport();
      if (imported === null) return; // cancelled, nothing to say
      // Stop what is running before the nodes go. These are the nodes being replaced, so the same
      // teardown a close does has to happen here or their PTYs outlive the work they belonged to.
      for (const n of nodesRef.current) {
        void terminalKill(n.id).catch(() => {
          // Best effort. Everything is being replaced either way.
        });
      }
      nodesRef.current = imported.nodes;
      edgesRef.current = imported.edges;
      setEdges(imported.edges);
      titleRef.current = imported.title;
      viewportRef.current = imported.viewport;
      seatRef.current = null;
      setSeat(null);
      setSeatAgent(null);
      // An imported workspace may reference an image that is not in this machine's library. It
      // draws as the plain background rather than erroring, the same fallback a removed library
      // file gets.
      wallpaperRef.current = imported.wallpaper;
      setWallpaper(imported.wallpaper);
      setNodes(imported.nodes);
      setTree(leaf("pane-0", imported.nodes[0]?.id ?? null));
      setFocusLeaf("pane-0");
      // The engine already wrote it to disk as part of importing, so the window is in step with the
      // file rather than one debounce behind it.
      unsaved.current = false;
      setSaveError(null);
    } catch (e) {
      setSaveError(`That workspace was not imported: ${String(e)}`);
    }
  }, []);

  // Picking a wallpaper applies immediately and rides the debounced save: the choice is one field
  // on the workspace, not its own persistence path.
  const pickWallpaper = useCallback(
    (w: Wallpaper) => {
      wallpaperRef.current = w;
      setWallpaper(w);
      scheduleSave();
    },
    [scheduleSave],
  );

  // Moving the seat is one write. Nothing is spawned or killed here: the seat is a role.
  const assignSeat = useCallback(
    (nodeId: string | null) => {
      seatRef.current = nodeId;
      setSeat(nodeId);
      scheduleSave();
    },
    [scheduleSave],
  );

  // ── the keyboard ────────────────────────────────────────────────────────────
  //
  // Every pane in this shell is a terminal that takes keys first, so these are captured on the
  // window the same way the quit shortcut is. A shortcut that only works when nothing has focus is
  // not a shortcut here: there is almost always a terminal with focus.
  //
  // This is the whole v0.2.0 set, and it is deliberately small: switch tab, walk the panes, split,
  // close a pane, reach the right column, quit. A command palette and drag-to-reorder are additions
  // to a shell that exists, and neither is cheaper to decide now.
  const showInFocused = useCallback((nodeId: string) => {
    setTree((cur) => setNode(cur, focusLeafRef.current, nodeId));
  }, []);

  const splitFocused = useCallback(() => {
    const id = `pane-${crypto.randomUUID()}`;
    setTree((cur) => splitLeaf(cur, focusLeafRef.current, id, "row"));
    setFocusLeaf(id);
  }, []);

  const closeFocusedPane = useCallback(() => {
    const going = focusLeafRef.current;
    const rest = leaves(treeRef.current).filter((l) => l.id !== going);
    if (rest.length === 0) return; // the last pane stays; there would be nowhere to go
    setTree((cur) => closeLeaf(cur, going));
    setFocusLeaf(rest[0]!.id);
  }, []);

  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.altKey) return;
      const take = () => {
        e.preventDefault();
        e.stopPropagation();
      };
      if (e.key === "q" || e.key === "Q") {
        // Ctrl+Q / Cmd+Q, because that is the key people press to leave an app. It raises the
        // ordinary close request, so the flush is the same one the title bar goes through.
        take();
        void closeIdentra();
        return;
      }
      if (e.key >= "1" && e.key <= "9") {
        const at = Number(e.key) - 1;
        const node = nodesRef.current[at];
        if (node === undefined) return;
        take();
        showInFocused(node.id);
        return;
      }
      if (e.key === "]" || e.key === "[") {
        take();
        setFocusLeaf((cur) =>
          stepLeaf(treeRef.current, cur, e.key === "]" ? 1 : -1),
        );
        return;
      }
      if (e.key === "\\") {
        take();
        if (e.shiftKey) closeFocusedPane();
        else splitFocused();
        return;
      }
      if (e.key === "e" || e.key === "E") {
        take();
        setRight((cur) => (cur === null ? "files" : null));
      }
    };
    window.addEventListener("keydown", key, true);
    return () => window.removeEventListener("keydown", key, true);
  }, [closeIdentra, showInFocused, splitFocused, closeFocusedPane]);

  // ── the command center ──────────────────────────────────────────────────────
  const [dispatch, setDispatch] = useState<DispatchState>({ kind: "idle" });
  // The seat is briefed once per session, in front of the first instruction it receives. Kept in a
  // ref rather than state because nothing renders from it and it must not be stale inside the async
  // dispatch below.
  const seatBriefed = useRef(false);

  const sendToSeat = useCallback(
    async (instruction: string) => {
      // Liveness, not membership: the seat is headless and never appears as a tab, so the only
      // question that means anything is whether its process is still there.
      const current = seatRef.current;
      const alive =
        current === null
          ? false
          : await terminalStatus(current)
              .then((s) => s !== null && s !== "exited")
              .catch(() => false);

      const plan = planSeat(
        current,
        alive,
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
        // The orchestrator is headless on purpose. It runs a real CLI, because that is the only
        // thing that can actually do the work, but it never becomes a tab: making it one turned the
        // command center into a remote control for a terminal the user then had to go and read. The
        // conversation belongs in the bar they typed into.
        const spec = (await agentsByKind()).get(plan.agentId);
        if (!spec || !spec.available) {
          setDispatch({
            kind: "failed",
            error: `${agent?.name ?? plan.agentId} is not installed or not on your PATH, so the instruction was not sent.`,
          });
          return;
        }
        nodeId = crypto.randomUUID();
        try {
          await terminalStart(
            nodeId,
            plan.agentId,
            spec.cmd,
            spec.args,
            null,
            SEAT_ROWS,
            SEAT_COLS,
          );
        } catch (e) {
          setDispatch({
            kind: "failed",
            error: `${spec.name} did not start, so the instruction was not sent: ${String(e)}`,
          });
          return;
        }
        // A new orchestrator is a new conversation, and the bar's pane is keyed on this id, so
        // assigning it is also what tears the last one's terminal down and mounts a fresh one.
        assignSeat(nodeId);
        setSeatAgent(spec.name);
        fresh = true;
        seatBriefed.current = false;
        // The CLI has a terminal but is still drawing its own first screen, and several of them
        // discard whatever is already pending when they take over the tty. A short settle costs one
        // beat on the first instruction of a session and saves silently losing it.
        await new Promise((r) => setTimeout(r, 1200));
      }

      setDispatch({ kind: "sending", note: "Sending to the orchestrator" });
      try {
        await terminalSend(
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
          note: "Sent. Its reply appears here as it works.",
        });
      } catch (e) {
        setDispatch({
          kind: "failed",
          error: `That did not reach the orchestrator: ${String(e)}`,
        });
      }
    },
    [assignSeat],
  );

  // What the seat is doing, shown next to the bar so the user does not have to read a scrolling
  // terminal to know whether anything came of what they typed.
  const [plan, setPlan] = useState<string | null>(null);
  const [seatAsking, setSeatAsking] = useState(false);

  // Polled rather than pushed, and only while a seat exists. The board is written by agents through
  // the bus and the seat's status is read from output timing, so neither has an event to subscribe
  // to. Two cheap reads every few seconds is the honest cost of showing this at all.
  useEffect(() => {
    if (seat === null) {
      setPlan(null);
      setSeatAsking(false);
      return;
    }
    let dropped = false;
    const poll = async () => {
      // Both are best effort. The board can be mid-write and the seat can be closed between the
      // check and the call, and neither is worth a visible error.
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

  // The first fact a workspace ever learns opens the right column once, on memory, so the moment
  // the promise becomes true is seen and not buried under the terminals. memory_reveal_once returns
  // true on exactly one call per workspace, ever, so this cannot re-fire on a later fact or a later
  // session; the ref only keeps it from asking the engine on every poll tick this session.
  const revealAsked = useRef(false);
  useEffect(() => {
    if (workspace === null) return;
    revealAsked.current = false;
    let dropped = false;
    const tick = async () => {
      // Its own catch: the receipt not answering is no reason to stop counting facts, which is
      // what the badge beside it reads from.
      const shook = await busHandshakes().catch(
        () => ({}) as Record<string, Handshake>,
      );
      if (!dropped) setHandshakes(shook);
      const list = await memoryList(50).catch(() => null);
      if (dropped || list === null) return;
      setMemoryCount(list.length);
      if (list.length > 0 && !revealAsked.current) {
        revealAsked.current = true;
        const first = await memoryRevealOnce().catch(() => false);
        if (first && !dropped) setRight("work");
      }
    };
    void tick();
    const timer = window.setInterval(() => void tick(), MEMORY_POLL_MS);
    return () => {
      dropped = true;
      window.clearInterval(timer);
    };
  }, [workspace]);

  // Dropping a file from the OS onto the shell opens it as a tab. This is the user's own door to
  // the viewer; the engine still refuses anything outside the workspace, and the pane shows that
  // refusal rather than this handler pre-judging it.
  const workspaceOpenRef = useRef(false);
  workspaceOpenRef.current = workspace !== null;
  useEffect(() => {
    const un = getCurrentWebview().onDragDropEvent((event) => {
      // Before a workspace is open there is nowhere to put a tab, and no folder for the containment
      // rule to mean anything against.
      if (event.payload.type !== "drop" || !workspaceOpenRef.current) return;
      for (const path of event.payload.paths) {
        addNode("file", path.split("/").pop() ?? path, path);
      }
    });
    return () => {
      void un.then((f) => f());
    };
  }, [addNode]);

  // A dev server announcing its address. A browser tab opens on it, wired to the server so the pair
  // reads as one thing. Asking twice stacks nothing: if a browser tab is already showing that
  // address, there is nothing left to offer.
  const openPreview = useCallback(
    (devId: string, url: string) => {
      if (nodesRef.current.some((n) => n.kind === "browser" && n.cwd === url)) {
        return;
      }
      // The user clicked the address badge, so this connection is theirs.
      const browserId = addNode("browser", "Preview", url);
      wire(devId, browserId, "you");
    },
    [addNode, wire],
  );

  // An agent asking the shell to change. The window is the single writer of its own state, so the
  // engine sends the request here rather than editing canvas.json underneath us, and we answer.
  // Every branch must reply exactly once: an agent is blocked on this until it hears back.
  //
  // This is also the one place a node's lock is enforced, and it is the right place: it is the only
  // door an agent has. The user's own grants never come through here, which is the intended
  // asymmetry.
  const applyCanvasCommand = useCallback(
    (cmd: CanvasCommand): CanvasResult => {
      const p = cmd.params;
      const locked = (id?: string) =>
        nodesRef.current.some((n) => n.id === id && n.locked);
      // Named, so the agent can tell the user which node it was and they can decide, rather than
      // just reporting that something was refused.
      const lockedReason = (id: string) => {
        const name = nodesRef.current.find((n) => n.id === id)?.title ?? id;
        return `${name} is locked, so an agent cannot connect to it. The person at the keyboard can unlock it or connect it themselves.`;
      };
      switch (cmd.action) {
        case "add_terminal": {
          const kind = typeof p.kind === "string" ? p.kind : "codex";
          const known = agentsRef.current.find((a) => a.id === kind);
          // Refuse rather than open a tab that can never run. The agent gets a reason it can act
          // on, which is better than a broken tab appearing in the user's shell.
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
          // Refuse before spawning, not after. Opening the tab and then failing to connect it would
          // leave a stray agent running that nobody asked for and nobody owns.
          if (
            typeof p.connectTo === "string" &&
            p.connectTo &&
            locked(p.connectTo)
          ) {
            return { ok: false, error: lockedReason(p.connectTo) };
          }
          const title =
            typeof p.title === "string" && p.title ? p.title : known.name;
          const id = addNode(kind, title);
          if (typeof p.connectTo === "string" && p.connectTo)
            wire(p.connectTo, id, "agent");
          return { ok: true, id };
        }
        case "connect_nodes": {
          const { from, to } = p as { from?: string; to?: string };
          const has = (id?: string) =>
            nodesRef.current.some((n) => n.id === id);
          // Checking for undefined here as well as membership is what narrows both to a string for
          // the rest of the branch.
          if (
            from === undefined ||
            to === undefined ||
            !has(from) ||
            !has(to)
          ) {
            return { ok: false, error: "one of those is not open here" };
          }
          if (from === to)
            return { ok: false, error: "a node cannot be connected to itself" };
          // Either end being locked is enough to refuse. An edge is the bus authorization and it
          // reads both ways, so connecting out of a locked node exposes it exactly as much as
          // connecting in.
          if (locked(from)) return { ok: false, error: lockedReason(from) };
          if (locked(to)) return { ok: false, error: lockedReason(to) };
          wire(from, to, "agent");
          return { ok: true, id: `${from}->${to}` };
        }
        case "add_note": {
          const text = typeof p.text === "string" ? p.text : "";
          if (!text.trim())
            return { ok: false, error: "a note needs some text" };
          return { ok: true, id: addNode("note", text) };
        }
        case "show_file": {
          const path = typeof p.path === "string" ? p.path : "";
          if (!path) return { ok: false, error: "a file tab needs a path" };
          const title =
            typeof p.title === "string" && p.title
              ? p.title
              : (path.split("/").pop() ?? "file");
          // Same lock rule as every other connection an agent asks for, checked before the tab
          // exists so a refusal leaves nothing behind.
          if (
            typeof p.connectTo === "string" &&
            p.connectTo &&
            locked(p.connectTo)
          ) {
            return { ok: false, error: lockedReason(p.connectTo) };
          }
          const id = addNode("file", title, path);
          if (typeof p.connectTo === "string" && p.connectTo)
            wire(p.connectTo, id, "agent");
          return { ok: true, id };
        }
        default:
          return {
            ok: false,
            error: `Identra does not know how to ${cmd.action}`,
          };
      }
    },
    [addNode, wire],
  );

  // One subscription for the life of the window, reading the current handler through a ref.
  //
  // Subscribing on `applyCanvasCommand` looked right and is a trap. Its identity changes whenever
  // anything it closes over does, and the unlisten is a promise, so a re-subscribe is: register the
  // new listener, then await the old one's teardown. A command arriving inside that window is
  // delivered to both, and both act on it — an agent's `add_terminal` becomes two tabs and its
  // `connect_nodes` connects twice, with no error anywhere to explain it.
  const applyRef = useRef(applyCanvasCommand);
  applyRef.current = applyCanvasCommand;
  useEffect(() => {
    const un = onCanvasCommand((cmd) => {
      let result: CanvasResult;
      try {
        result = applyRef.current(cmd);
      } catch (e) {
        // Never leave the agent hanging on our bug: it waits on this reply.
        result = { ok: false, error: String(e) };
      }
      void canvasCommandResult(cmd.requestId, result);
    });
    return () => {
      void un.then((f) => f());
    };
  }, []);

  // Held in state rather than read off a node, because the seat is not one. It lives for the
  // session only, which is all it needs to.
  const seatName = seat === null ? null : seatAgent;

  if (!workspace) {
    return (
      <WorkspacePicker
        onOpen={(w) => void openWorkspace(w)}
        onClose={() => void closeIdentra()}
      />
    );
  }

  const paneCount = leaves(tree).length;

  return (
    <div className="identra-root identra-shell">
      {/* Behind the columns rather than under nodes: it shows at the edges and through the gaps,
          which is as much of a wallpaper as a shell has room for. data-scrim pulls a user image
          toward the app background; the built-ins and swatches are curated dark values. */}
      <div
        className="identra-wallpaper"
        data-scrim={needsScrim(wallpaper) || undefined}
        style={{ background: backgroundCss(wallpaper, convertFileSrc) }}
      />

      <aside
        className="identra-side"
        onContextMenu={(e) => {
          // The wallpaper had no home once the canvas went. Here is the one surface in the shell
          // that is the app rather than someone's content, so it is where right-clicking still
          // means "change how this looks".
          e.preventDefault();
          setWallMenu({ x: e.clientX, y: e.clientY });
        }}
      >
        <div className="identra-side__head">
          <img className="identra-logo" src={logo} alt="" />
          <WorkspaceMenu
            workspace={workspace}
            onOpen={(w) => void openWorkspace(w)}
            onRenamed={setWorkspace}
            onDeleted={() => {
              // Back to the picker. The workspace under us is gone, so there is nothing to show
              // and nothing to save into.
              setWorkspace(null);
              setNodes([]);
              setEdges([]);
              nodesRef.current = [];
              edgesRef.current = [];
            }}
          />
        </div>

        <HandshakeLine handshakes={handshakes} nodes={nodes} />

        <div className="identra-side__section">Open an agent</div>
        <div className="identra-side__agents">
          {agents.map((a) => {
            const state = a.available
              ? a.logged_in
                ? "ready"
                : "setup"
              : "missing";
            return (
              <button
                key={a.id}
                className="identra-side__agent"
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
                  // A signed-in agent just opens. One that is installed but not signed in would
                  // drop the user into a raw login prompt with no idea why, so I name what is about
                  // to happen first. The tab still runs the real CLI, which is where the sign-in
                  // lives, so this is a heads-up and not a second login path to keep in step.
                  if (
                    state === "setup" &&
                    !window.confirm(
                      `${a.name} is installed but not signed in.\n\nOpening it will start its own sign-in in the tab. Follow the prompts there, then the dot turns green.`,
                    )
                  ) {
                    return;
                  }
                  addNode(a.id, a.name);
                }}
              >
                <AgentIcon kind={a.id} className="identra-side__tile" />
                <span className="identra-side__label">{a.name}</span>
                <span className="identra-side__dot" data-state={state} />
              </button>
            );
          })}
          <button
            className="identra-side__agent"
            data-state="ready"
            title="Open a web view"
            onClick={() =>
              addNode("browser", "Browser", "http://localhost:1420")
            }
          >
            <AgentIcon kind="browser" className="identra-side__tile" />
            <span className="identra-side__label">Browser</span>
          </button>
          {/* One dev server per workspace: the button is the way to start it, so it goes while one
              is open. Stopping is closing the tab, like everything else. */}
          {devCmd !== null && !nodes.some((n) => n.kind === "dev") && (
            <button
              className="identra-side__agent"
              data-state="ready"
              title={`Start the dev server: ${devCmd.join(" ")}`}
              onClick={() => addNode("dev", "Dev server")}
            >
              <AgentIcon kind="dev" className="identra-side__tile" />
              <span className="identra-side__label">Dev server</span>
            </button>
          )}
        </div>

        <div className="identra-side__foot">
          <button
            className="identra-side__btn"
            onClick={() => setSettingsOpen((v) => !v)}
            data-on={settingsOpen}
          >
            Settings
          </button>
          <button
            className="identra-side__btn"
            onClick={() => void exportCanvas()}
            title="Save this workspace to a file"
          >
            Export
          </button>
          <button
            className="identra-side__btn"
            onClick={() => void importCanvas()}
            title="Replace this workspace with one from a file"
          >
            Import
          </button>
          {/* Two different exits, and they were one control until someone pointed out they are not
              the same act at all. Leaving a workspace is going back to the list of your projects;
              leaving Identra is ending every agent on the machine. Closing Identra is not offered
              here: inside a workspace the thing you usually want is the other workspace, and the
              button that ends every agent should not sit one pixel from the one that lists your
              projects. ctrl+Q still works from anywhere. */}
          <button
            className="identra-side__btn"
            onClick={() => void goHome()}
            title="Back to your workspaces. The agents in this one keep running."
          >
            Home
          </button>
        </div>
      </aside>

      <main className="identra-centre">
        {saveError !== null && (
          // It stays until a save works. Work that is not on disk is not a thing to mention once
          // and then hide: everything from here is work that will not be there tomorrow, and the
          // user is the only one who can do anything about a full disk or a read-only folder.
          <div className="identra-save-error" role="alert">
            <strong>This workspace is not being saved.</strong> {saveError}
          </div>
        )}
        {canvasNotice && (
          // Said once, on the first open after the canvas went away. Ten months of arrangement
          // disappearing into a layout the user did not choose, with nothing saying it was
          // deliberate, reads as a corrupted file rather than a new version.
          <div className="identra-notice">
            <span>
              Your agents are still here, as tabs. The arrangement is not — that
              is deliberate. Everything you connected stayed connected.
            </span>
            <button onClick={dismissCanvasNotice}>Got it</button>
          </div>
        )}

        <div className="identra-tabs" role="tablist">
          {nodes.map((n, i) => (
            <Tab
              key={n.id}
              node={n}
              index={i}
              active={leaves(tree).some((l) => l.nodeId === n.id)}
              onOpen={() => showInFocused(n.id)}
              onClose={() => closeNode(n.id)}
            />
          ))}
          {nodes.length > 0 && (
            <button
              className="identra-tabs__split"
              title={`Split the focused pane (${MOD_LABEL.replace("K", "\\")})`}
              onClick={splitFocused}
            >
              Split
            </button>
          )}
        </div>

        <div className="identra-panes">
          {nodes.length === 0 ? (
            // A blank shell reads as a broken app. With no agent installed the sidebar is all
            // disabled, so the usual hint would point at controls you cannot use.
            noAgentsInstalled(agents) ? (
              <Onboarding agents={agents} onRecheck={recheckAgents} />
            ) : (
              <div className="identra-empty">
                <p className="identra-empty__lead">This workspace is empty.</p>
                {/* Two ways in, in the order they are worth trying. Saying what you want is the
                    whole product and it is one keystroke away, so it leads; the sidebar is the
                    manual path for when you already know which agent you want. */}
                <p className="identra-empty__hint">
                  Say what you want done in the bar below — press{" "}
                  <kbd className="identra-empty__kbd">{MOD_LABEL}</kbd> from
                  anywhere — and an orchestrator breaks the work up and opens
                  the agents it needs.
                </p>
                <p className="identra-empty__hint">
                  Or open one yourself from the left. Open a second, connect
                  them, and they can split the work between themselves.
                </p>
              </div>
            )
          ) : (
            <PaneTreeView
              pane={tree}
              nodes={nodes}
              focusLeaf={focusLeaf}
              onFocusLeaf={setFocusLeaf}
              onClosePane={(id) => {
                const rest = leaves(tree).filter((l) => l.id !== id);
                if (rest.length === 0) return;
                setTree((cur) => closeLeaf(cur, id));
                setFocusLeaf(rest[0]!.id);
              }}
              closable={paneCount > 1}
              onSetCwd={setNodeCwd}
              onPreviewUrl={openPreview}
            />
          )}
        </div>

        {/* Hidden until an agent exists to run it: on a machine with nothing installed the
            onboarding panel is the thing to read, and a command bar that can only fail is worse
            than no command bar. */}
        {!noAgentsInstalled(agents) && (
          <CommandBar
            seatName={seatName}
            state={dispatch}
            plan={plan}
            seatId={seat}
            awaitingAnswer={seatAsking}
            // The last word on a dispatch, wherever it went wrong.
            //
            // `sending` disables the input, which is right while an instruction is genuinely on its
            // way and a disaster if it latches: the bar is the only way to talk to the orchestrator,
            // so a stuck `sending` is an app that has to be restarted to accept another word.
            // Catching at the one place every path returns through is what makes that impossible.
            onSubmit={(instruction) =>
              void sendToSeat(instruction).catch((e: unknown) =>
                setDispatch({
                  kind: "failed",
                  error: `The instruction did not go anywhere: ${String(e)}`,
                }),
              )
            }
          />
        )}
      </main>

      <aside className="identra-right" data-open={right !== null || undefined}>
        <div className="identra-right__tabs">
          <button
            data-on={right === "work"}
            onClick={() => setRight((cur) => (cur === "work" ? null : "work"))}
            title="What your agents are working on, and what this project has learned"
          >
            Work
            {/* The ambient signal that memory is accumulating: a count on the toggle, no toast
                stream. It reads whether or not the column is open. */}
            {memoryCount > 0 && (
              <span className="identra-right__badge">{memoryCount}</span>
            )}
          </button>
          <button
            data-on={right === "files"}
            onClick={() =>
              setRight((cur) => (cur === "files" ? null : "files"))
            }
            title="Browse and search this workspace's files"
          >
            Files
          </button>
          <button
            data-on={right === "connections"}
            onClick={() =>
              setRight((cur) => (cur === "connections" ? null : "connections"))
            }
            title="Which agents can read each other's work, and who allowed it"
          >
            Links
            {/* Counted whether or not the column is open, for the same reason the memory badge is:
                an agent can grant itself one of these, and a number that only appears once you go
                looking is not a number that tells you anything happened. */}
            {edges.length > 0 && (
              <span className="identra-right__badge">{edges.length}</span>
            )}
          </button>
        </div>
        {right === "work" && (
          <WorkPanel nodes={nodes} onClose={() => setRight(null)} />
        )}
        {right === "connections" && (
          <ConnectionsPanel
            nodes={nodes}
            edges={edges}
            onRevoke={revoke}
            onClose={() => setRight(null)}
          />
        )}
        {right === "files" && (
          <FilesPanel
            onClose={() => setRight(null)}
            onOpenFile={(rel, name) => {
              // The panel speaks workspace-relative; the viewer stores the absolute path, same as
              // every other door to it, so a saved workspace needs no second path shape.
              addNode("file", name, `${workspace.path}/${rel}`);
            }}
          />
        )}
      </aside>

      {wallMenu !== null && (
        <WallpaperPicker
          current={wallpaper}
          at={wallMenu}
          onPick={pickWallpaper}
          onClose={() => setWallMenu(null)}
        />
      )}
      {settingsOpen && <SettingsPanel onClose={() => setSettingsOpen(false)} />}
    </div>
  );
}

// The receipt, in the sidebar under the workspace name: what the last agent to connect was handed.
//
// It says **sent**, and it has to keep saying sent. The MCP initialize response goes to the agent
// and never comes back to us, so whether the model read a word of it is not observable — over ACP
// it is not observable even in principle, because the protocol does not return the server's
// initialize response to the client at all. "Knows 7 facts" would read better and would be a claim
// nobody measured.
//
// The most recent one rather than a list. This is a glance that answers "did the thing happen",
// and a column of counts is a thing to study. The full list is one hover away.
function HandshakeLine({
  handshakes,
  nodes,
}: {
  handshakes: Record<string, Handshake>;
  nodes: CanvasNode[];
}) {
  const now = Math.floor(Date.now() / 1000);
  const named = Object.entries(handshakes)
    .map(([id, h]) => ({
      // A handshake from an agent that has since been closed still counts as evidence, so it keeps
      // its place with whatever name is left rather than being dropped for lack of a tab.
      name: nodes.find((n) => n.id === id)?.title ?? "an agent",
      ...h,
    }))
    .sort((a, b) => b.at - a.at);
  const last = named[0];
  if (last === undefined) return null;
  return (
    <div
      className="identra-side__handshake"
      title={named
        .map(
          (h) => `${h.name}: sent ${h.facts} fact${h.facts === 1 ? "" : "s"}`,
        )
        .join("\n")}
    >
      {/* Zero is worth saying rather than hiding. "Connected and there was nothing to tell it" and
          "has not connected" look identical to a person, and only one of them means this works. */}
      {last.facts === 0
        ? `Sent no facts to ${last.name} — nothing learned here yet`
        : `Sent ${last.facts} fact${last.facts === 1 ? "" : "s"} to ${last.name}`}
      <span className="identra-side__handshake-when">{ago(last.at, now)}</span>
    </div>
  );
}

// One tab. It is the node's whole existence in the shell: open it into a pane, or close it for
// good. The dot is here as well as on the pane because a tab you are not looking at is exactly the
// one you want to know has stopped.
function Tab({
  node,
  index,
  active,
  onOpen,
  onClose,
}: {
  node: CanvasNode;
  index: number;
  active: boolean;
  onOpen: () => void;
  onClose: () => void;
}) {
  const terminal =
    node.kind !== "browser" && node.kind !== "file" && node.kind !== "note";
  return (
    <div className="identra-tab" data-active={active || undefined} role="tab">
      <button className="identra-tab__open" onClick={onOpen}>
        {terminal && <TabDot nodeId={node.id} />}
        <AgentIcon kind={node.kind} className="identra-tab__icon" />
        <span className="identra-tab__title">{node.title || node.kind}</span>
        {index < 9 && <span className="identra-tab__num">{index + 1}</span>}
      </button>
      <button
        className="identra-tab__close"
        title={
          node.kind === "dev"
            ? "Stop the dev server and close this tab"
            : "Stop this agent and close this tab. Its conversation is forgotten."
        }
        onClick={onClose}
      >
        &times;
      </button>
    </div>
  );
}

function TabDot({ nodeId }: { nodeId: string }) {
  const state = useNodeState(nodeId);
  return <span className="identra-node__dot" data-state={state} />;
}

// The split tree, drawn. Flex the whole way down, so a split is a box beside a box and the browser
// does the arithmetic. Ratios are not stored or dragged in v0.2.0: an even split is what a split
// means until someone has asked for it to mean something else.
function PaneTreeView({
  pane,
  nodes,
  focusLeaf,
  onFocusLeaf,
  onClosePane,
  closable,
  onSetCwd,
  onPreviewUrl,
}: {
  pane: PaneTree;
  nodes: CanvasNode[];
  focusLeaf: string;
  onFocusLeaf: (id: string) => void;
  onClosePane: (id: string) => void;
  closable: boolean;
  onSetCwd: (nodeId: string, cwd: string) => void;
  onPreviewUrl: (nodeId: string, url: string) => void;
}) {
  if (pane.kind === "split") {
    return (
      <div className="identra-split" data-dir={pane.dir}>
        {[pane.a, pane.b].map((half) => (
          <PaneTreeView
            key={half.id}
            pane={half}
            nodes={nodes}
            focusLeaf={focusLeaf}
            onFocusLeaf={onFocusLeaf}
            onClosePane={onClosePane}
            closable={closable}
            onSetCwd={onSetCwd}
            onPreviewUrl={onPreviewUrl}
          />
        ))}
      </div>
    );
  }
  const node = nodes.find((n) => n.id === pane.nodeId);
  if (node === undefined) {
    return (
      <div
        className="identra-pane identra-pane--empty"
        data-focused={pane.id === focusLeaf || undefined}
        onMouseDownCapture={() => onFocusLeaf(pane.id)}
      >
        <p>Pick a tab to show it here.</p>
      </div>
    );
  }
  return (
    <Pane
      // Keyed on both, so moving a node to the other half of a split remounts its terminal into the
      // box it is actually in rather than carrying the old one's size across.
      key={`${pane.id}:${node.id}`}
      node={node}
      focused={pane.id === focusLeaf}
      onFocus={() => onFocusLeaf(pane.id)}
      onClosePane={() => onClosePane(pane.id)}
      closable={closable}
      onSetCwd={onSetCwd}
      onPreviewUrl={onPreviewUrl}
    />
  );
}

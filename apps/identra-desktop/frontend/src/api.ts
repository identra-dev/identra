// Typed wrappers over the Tauri commands in src-tauri/src/lib.rs. Keep the shapes in sync
// with the Rust structs (identra-core canvas + the command layer).
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type AgentInfo = {
  id: string;
  name: string;
  path: string;
  available: boolean;
  logged_in: boolean;
  cmd: string;
  args: string[];
  // Whether Identra knows how to put this agent on the context bus. It is the capability the
  // orchestrator seat is chosen on, because an agent that cannot spawn a helper, wire it, or reach
  // the board cannot orchestrate anything.
  bus_wired: boolean;
};

export type CanvasNode = {
  id: string;
  kind: string;
  x: number;
  y: number;
  width: number;
  height: number;
  title: string;
  cwd: string | null;
  // The user has closed this node to changes made by agents. It stops agents wiring anything to it,
  // which matters more than it sounds: an edge is the bus authorization, so refusing new edges is
  // what keeps this node's transcript out of reach of an agent that would wire itself in to read it.
  locked: boolean;
};

export type Edge = { id: string; source: string; target: string };

export type Viewport = { x: number; y: number; zoom: number };

// The canvas background. A reference, never image bytes: a built-in background by id, a flat hex
// color, or an absolute path into the shared wallpaper library. Mirrors the tagged enum in
// identra-core canvas.rs.
export type Wallpaper =
  | { kind: "yaru"; value: string }
  | { kind: "color"; value: string }
  | { kind: "image"; value: string };

export type Canvas = {
  nodes: CanvasNode[];
  edges: Edge[];
  viewport: Viewport;
  title: string;
  // The node holding the orchestrator seat, or null if the command center has not been opened here.
  // One id on the canvas rather than a flag per node, so "at most one seat" is a fact about the
  // shape of the data rather than a rule to remember.
  seat: string | null;
  wallpaper: Wallpaper;
};

// A workspace is a folder: `path` is where it really lives, `slug` is the folder name and the id,
// `title` is what the user reads. The canvas rides along so the picker can draw a thumbnail of
// the board without a second command; it is a snapshot from listing time, and opening still loads
// fresh from the engine.
export type WorkspaceMeta = {
  slug: string;
  title: string;
  path: string;
  canvas: Canvas;
};

// Identra made it (under the workspaces root) versus you opened a folder you already had. The two
// behave differently in exactly one place that matters: Identra's are Identra's to delete, and
// yours are only ever forgotten. An adopted folder carries its own path as its id, which is what
// tells them apart.
export const isAdopted = (w: WorkspaceMeta) => w.slug === w.path;

export type Snapshot = { data: number[]; lastSeq: number };
export type OutputEvent = { id: string; seq: number; data: number[] };
// `code` is null when the agent was killed by a signal rather than exiting on its own.
export type ExitEvent = { id: string; code: number | null };

export const detectAgents = () => invoke<AgentInfo[]>("detect_agents");

// Which agent the engine would put in the orchestrator seat here, by id, or null when nothing
// installed can hold it. Ranked on capability in the engine, so the UI does not carry a second
// opinion about it. This is a default the UI offers, never a decision it makes for the user.
export const defaultOrchestrator = () =>
  invoke<string | null>("default_orchestrator");

// What the seat agent is told before the user's first instruction. Read from the engine rather than
// written here so it stays next to the workspace guide it builds on.
export const seatBrief = () => invoke<string>("seat_brief");

// The install/login state is fixed for a session, so probe once and let every node share the
// result. A node looks itself up by kind to learn which binary and args to spawn.
let agentCache: Promise<Map<string, AgentInfo>> | null = null;
export const agentsByKind = (): Promise<Map<string, AgentInfo>> => {
  if (!agentCache) {
    agentCache = detectAgents().then(
      (list) => new Map(list.map((a) => [a.id, a])),
    );
  }
  return agentCache;
};

// Detection is fixed for a session on purpose, but a first-run user who installs an agent while the
// app is open should not have to relaunch for the dock to notice. This drops the cached probe and
// runs a fresh one; callers update their own agent state from the returned list, and the next node
// that looks itself up will re-probe rather than read the stale map.
export const refreshAgents = (): Promise<AgentInfo[]> => {
  agentCache = null;
  return detectAgents();
};

// Whether to show the first-run "install an agent" panel. True only once detection has answered and
// every known agent is missing. The length guard matters: before the first probe resolves the list
// is empty for a different reason, and without it the panel would flash on every launch.
export const noAgentsInstalled = (list: AgentInfo[]): boolean =>
  list.length > 0 && list.every((a) => !a.available);

// `kind` is the agent id. The engine uses it to add that CLI's bus wiring at launch, which is the
// only moment it can happen: every agent reads its MCP servers once, on startup.
export const terminalStart = (
  id: string,
  kind: string,
  cmd: string,
  args: string[],
  cwd: string | null,
  rows: number,
  cols: number,
) => invoke<void>("terminal_start", { id, kind, cmd, args, cwd, rows, cols });

export const terminalInput = (id: string, data: string) =>
  invoke<void>("terminal_input", { id, data });

export const terminalResize = (id: string, rows: number, cols: number) =>
  invoke<void>("terminal_resize", { id, rows, cols });

export const terminalSnapshot = (id: string) =>
  invoke<Snapshot | null>("terminal_snapshot", { id });

// What the engine thinks a node is doing. The node tracks running and exited itself, from events,
// so the reason to ask is the one thing events cannot carry: whether the quiet it just fell into is
// an agent that finished or an agent waiting for an answer. Null means there is no live terminal
// under that id, which happens if the node was killed between settling and asking.
export type TerminalStatus = "running" | "idle" | "needs-input" | "exited";

export const terminalStatus = (id: string) =>
  invoke<TerminalStatus | null>("terminal_status", { id });

export const terminalKill = (id: string) =>
  invoke<void>("terminal_kill", { id });

export const canvasLoad = () => invoke<Canvas>("canvas_load");

export const canvasSave = (canvas: Canvas) =>
  invoke<void>("canvas_save", { canvas });

// Both open a file dialog, so both resolve to a "nothing happened" value when the user cancels:
// false from export, null from import. A cancelled dialog is not an error and must never be shown
// as one. The board is passed to export rather than read from disk so what lands in the file is
// what is on screen, including anything the debounced save has not written yet.
export const canvasExport = (canvas: Canvas) =>
  invoke<boolean>("canvas_export", { canvas });

export const canvasImport = () => invoke<Canvas | null>("canvas_import");

// The shared board the agents claim work from, and what the project has learned. Both are written
// by agents through the bus; these read them so the human can see the same thing they do.
export type Task = {
  id: number;
  description: string;
  claimedBy: string | null;
  done: boolean;
  note: string | null;
  blockedBy: number[];
};

export type Memory = {
  id: number;
  content: string;
  created_at: number;
};

export const boardList = () => invoke<Task[]>("board_list");

export const memoryList = (limit?: number) =>
  invoke<Memory[]>("memory_list", { limit: limit ?? null });

export const memorySearch = (query: string, limit?: number) =>
  invoke<Memory[]>("memory_search", { query, limit: limit ?? null });

// The command that starts this workspace's dev server, split into argv, or null when the project
// does not declare one. Read per workspace: the Run control exists exactly when this answers.
export const devCommand = () => invoke<string[] | null>("dev_command");

// One row of the Files panel, workspace-relative, mirroring identra-core files.rs.
export type FileEntry = {
  name: string;
  path: string;
  dir: boolean;
  size: number;
};

// A search hit: a name match carries no line, a content match carries the line and a snippet.
export type FileHit = {
  path: string;
  line: number | null;
  snippet: string | null;
};

export const filesList = (rel: string) =>
  invoke<FileEntry[]>("files_list", { rel });

export const filesSearch = (query: string) =>
  invoke<FileHit[]>("files_search", { query });

export const fileReveal = (rel: string) =>
  invoke<void>("file_reveal", { rel });

// What the file viewer renders, mirroring identra-core fileview.rs. Image bytes arrive as a
// plain array and become a blob URL on this side, which keeps base64 out of both.
export type FileView =
  | { kind: "text"; name: string; text: string }
  | { kind: "image"; name: string; bytes: number[] }
  | { kind: "binary"; name: string; size: number }
  | { kind: "toobig"; name: string; size: number };

// Refuses any path that does not resolve inside the active workspace; the rejection carries the
// reason and the viewer shows it.
export const fileRead = (path: string) =>
  invoke<FileView>("file_read", { path });

// What is true of this machine, as opposed to of one workspace. Mirrors identra-core settings.rs.
export type Settings = {
  // Recall by meaning: on fetches a local model once (about 130MB), off matches by words and
  // never touches the network. Read by the engine once per process, so a change lands at the
  // next launch.
  embeddings: boolean;
};

export const settingsGet = () => invoke<Settings>("settings_get");

export const settingsSet = (settings: Settings) =>
  invoke<void>("settings_set", { settings });

// The shared wallpaper library: images added once, usable from any workspace. Listing returns
// absolute paths; the canvas layer turns each into an asset URL when it draws.
export const wallpapersList = () => invoke<string[]>("wallpapers_list");

// Opens a native image picker and copies the choice into the library. Null is a cancelled dialog,
// which is an answer, not an error.
export const wallpaperAdd = () => invoke<string | null>("wallpaper_add");

// The engine refuses any path that is not directly inside the library, so this cannot be talked
// into deleting anything else.
export const wallpaperRemove = (path: string) =>
  invoke<void>("wallpaper_remove", { path });

export const workspaceList = () => invoke<WorkspaceMeta[]>("workspace_list");

// Creating a workspace makes the folder, writes a blank canvas into it, and makes it active. The
// same folder is where the agents in it will run.
export const workspaceCreate = (title?: string) =>
  invoke<WorkspaceMeta>("workspace_create", { title: title ?? null });

export const workspaceOpen = (slug: string) =>
  invoke<Canvas>("workspace_open", { slug });

// Clone a repository into the workspaces root and open it. Takes as long as the network takes;
// the caller owns saying "cloning" meanwhile. A failure carries git's own words.
export const workspaceClone = (url: string) =>
  invoke<WorkspaceMeta>("workspace_clone", { url });

// Folders you already had, opened as workspaces. This is how Identra works on real code rather than
// only on scratch workspaces it made itself.
export const workspaceRecents = () =>
  invoke<WorkspaceMeta[]>("workspace_recents");

// Opens a native folder picker and adopts what you choose. The path never comes from here: the
// picker is the authorization, so the whole thing is one call.
export const workspacePickFolder = () =>
  invoke<WorkspaceMeta | null>("workspace_pick_folder");

export const workspaceOpenRecent = (path: string) =>
  invoke<Canvas>("workspace_open_recent", { path });

export const workspaceForgetRecent = (path: string) =>
  invoke<void>("workspace_forget_recent", { path });

// Renaming moves the folder, so the returned meta carries the new slug and path. Anything holding
// the old one is stale.
export const workspaceRename = (slug: string, title: string) =>
  invoke<WorkspaceMeta>("workspace_rename", { slug, title });

// Takes the folder and everything in it, which is the user's work, not just a layout file. Ask
// first.
export const workspaceDelete = (slug: string) =>
  invoke<void>("workspace_delete", { slug });

export const onOutput = (cb: (e: OutputEvent) => void): Promise<UnlistenFn> =>
  listen<OutputEvent>("terminal://output", (evt) => cb(evt.payload));

// Fires once per node, when its agent is gone. Without it a finished agent looks like a thinking
// one forever, because silence is all the node would otherwise have to go on.
export const onExit = (cb: (e: ExitEvent) => void): Promise<UnlistenFn> =>
  listen<ExitEvent>("terminal://exit", (evt) => cb(evt.payload));

// A canvas change an agent asked for. The canvas is the only writer of its own state, so the engine
// asks rather than writing, and waits for the reply keyed by requestId.
export type CanvasCommand = {
  requestId: string;
  action: string;
  params: Record<string, unknown>;
};

// What the canvas says it did. `id` names whatever was created, so the agent can talk about it.
export type CanvasResult =
  { ok: true; id?: string } | { ok: false; error: string };

export const onCanvasCommand = (
  cb: (c: CanvasCommand) => void,
): Promise<UnlistenFn> =>
  listen<CanvasCommand>("canvas://command", (evt) => cb(evt.payload));

export const canvasCommandResult = (requestId: string, result: CanvasResult) =>
  invoke<void>("canvas_command_result", { requestId, result });

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
};

export type Edge = { id: string; source: string; target: string };

export type Viewport = { x: number; y: number; zoom: number };
export type Canvas = {
  nodes: CanvasNode[];
  edges: Edge[];
  viewport: Viewport;
  title: string;
};

// A workspace is a folder: `path` is where it really lives, `slug` is the folder name and the id,
// `title` is what the user reads.
export type WorkspaceMeta = { slug: string; title: string; path: string };

export type Snapshot = { data: number[]; lastSeq: number };
export type OutputEvent = { id: string; seq: number; data: number[] };
// `code` is null when the agent was killed by a signal rather than exiting on its own.
export type ExitEvent = { id: string; code: number | null };

export const detectAgents = () => invoke<AgentInfo[]>("detect_agents");

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

export const terminalKill = (id: string) =>
  invoke<void>("terminal_kill", { id });

export const canvasLoad = () => invoke<Canvas>("canvas_load");

export const canvasSave = (canvas: Canvas) =>
  invoke<void>("canvas_save", { canvas });

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

export const workspaceList = () => invoke<WorkspaceMeta[]>("workspace_list");

// Creating a workspace makes the folder, writes a blank canvas into it, and makes it active. The
// same folder is where the agents in it will run.
export const workspaceCreate = (title?: string) =>
  invoke<WorkspaceMeta>("workspace_create", { title: title ?? null });

export const workspaceOpen = (slug: string) =>
  invoke<Canvas>("workspace_open", { slug });

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

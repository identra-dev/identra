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
export type Canvas = { nodes: CanvasNode[]; edges: Edge[]; viewport: Viewport };

export type Snapshot = { data: number[]; lastSeq: number };
export type OutputEvent = { id: string; seq: number; data: number[] };

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

export const terminalStart = (
  id: string,
  cmd: string,
  args: string[],
  cwd: string | null,
  rows: number,
  cols: number,
) => invoke<void>("terminal_start", { id, cmd, args, cwd, rows, cols });

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

// Point codex at the running context bus by writing its MCP config. Called when the first edge is
// drawn, since codex reads its server list only at launch.
export const writeAgentMcpConfig = () => invoke<void>("write_agent_mcp_config");

export const onOutput = (cb: (e: OutputEvent) => void): Promise<UnlistenFn> =>
  listen<OutputEvent>("terminal://output", (evt) => cb(evt.payload));

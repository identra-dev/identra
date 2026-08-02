// What one node is doing, for the dot next to its name.
//
// Four honest states. Output means it is working; 1.5s of quiet settles it. Exit is the only one
// that cannot be inferred from timing, so the engine says it: without that, an agent that finished
// looks exactly like one that is thinking, forever. And when it settles, quiet splits in two —
// finished, or waiting on an answer — which the engine works out from the transcript, because the
// difference is in what was printed and not in the timing.
//
// This came out of the canvas node, where it was tangled with the terminal it drew. A pane can be
// closed and reopened over a process that keeps running, so the state has to be readable from
// scratch at any moment rather than accumulated from the moment a view happened to mount. That is
// what the seeding read is for: a tab you come back to after an agent exited must not say ready.
import { useEffect, useState } from "react";
import { onExit, onOutput, terminalStatus } from "./api";

export type NodeState = "ready" | "running" | "needs-input" | "exited";

// How long a node has to stay quiet before it stops reading as busy. The engine uses the same
// threshold when it answers `terminal_status`, so the two agree about what quiet means.
const SETTLE_MS = 1500;

export function useNodeState(nodeId: string): NodeState {
  const [state, setState] = useState<NodeState>("ready");

  useEffect(() => {
    let running = false;
    let exited = false;
    let dropped = false;
    let idleTimer: number | undefined;

    // What the engine already knows, asked once. A pane that mounts over a node which exited an
    // hour ago has no event coming to tell it so.
    void terminalStatus(nodeId)
      .then((seed) => {
        if (dropped || seed === null || running || exited) return;
        setState(seed === "idle" ? "ready" : seed);
        if (seed === "exited") exited = true;
      })
      .catch(() => {
        // Nothing behind this id yet. Ready is already right.
      });

    const settle = () => {
      running = false;
      setState("ready");
      // Settling is the one moment worth asking what this quiet means. Ready goes on first and is
      // corrected after, so the dot never waits on IPC to stop looking busy. Anything other than
      // needs-input leaves it alone: the engine uses the same threshold, so it can still say
      // "running" here by a hair, and the local timer should win that tie.
      void terminalStatus(nodeId)
        .then((status) => {
          if (dropped || exited || running) return;
          if (status === "needs-input") setState("needs-input");
        })
        .catch(() => {
          // Killed between settling and asking. Ready is already right.
        });
    };

    const unlisten = onOutput((e) => {
      if (e.id !== nodeId) return;
      if (exited) return; // a dead agent producing bytes is drain, not life
      if (!running) {
        running = true;
        setState("running");
      }
      window.clearTimeout(idleTimer);
      idleTimer = window.setTimeout(settle, SETTLE_MS);
    });

    const unlistenExit = onExit((e) => {
      if (e.id !== nodeId) return;
      window.clearTimeout(idleTimer); // it cannot go back to running now
      running = false;
      exited = true;
      setState("exited");
    });

    return () => {
      dropped = true;
      window.clearTimeout(idleTimer);
      void unlisten.then((un) => un());
      void unlistenExit.then((un) => un());
    };
  }, [nodeId]);

  return state;
}

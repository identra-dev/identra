// The decisions the command bar has to make before it can send anything, kept out of the component
// so they can be read and tested on their own. The component does the IO; this says what the IO
// should be.

import type { Task } from "./api";

// What has to happen before an instruction can be dispatched.
//
// Three outcomes and no fourth: talk to the seat that is already there, stand one up first, or
// admit that this machine has nothing that could hold the seat. The last one is a real state, not
// an error path to swallow: a user with no agent installed types into the bar and deserves to be
// told why nothing happened.
export type SeatPlan =
  | { kind: "use"; nodeId: string }
  | { kind: "create"; agentId: string }
  | { kind: "unavailable" };

// `seat` is the id the canvas remembers, `nodeIds` is what is actually on it, and `defaultAgentId`
// is the engine's capability-ranked pick (null when nothing installed can be wired to the bus).
//
// The seat is only honoured when its node is still on the canvas. A stale id is the normal case
// after someone closes the seat node, so it is treated as "no seat" rather than as a broken state:
// the bar quietly stands a new one up instead of failing at the user.
export function planSeat(
  seat: string | null,
  nodeIds: readonly string[],
  defaultAgentId: string | null,
): SeatPlan {
  if (seat !== null && nodeIds.includes(seat)) {
    return { kind: "use", nodeId: seat };
  }
  if (defaultAgentId === null) return { kind: "unavailable" };
  return { kind: "create", agentId: defaultAgentId };
}

// What actually gets typed into the seat's terminal.
//
// The brief goes in front of the very first instruction of a session rather than being sent on its
// own, because a CLI that is still starting up can discard input it has not begun reading yet, and
// one write is one thing to get right instead of two. After that the user's words go through
// untouched: the agent has already been told how to work here, and repeating it every time would
// spend context on something it already knows.
//
// The trailing carriage return is what submits the line, the same as pressing enter in the node.
export function composeDispatch(
  brief: string,
  instruction: string,
  seatIsNew: boolean,
): string {
  const body = seatIsNew ? `${brief}\n\n${instruction}` : instruction;
  return `${body}\r`;
}

// The shape of the work the seat has broken an instruction into.
//
// This is read off the shared board rather than asked of the seat, and that is the point: the board
// is what the agents actually coordinate through, so a plan drawn from it is what is really
// happening rather than what an agent said it would do.
export type Plan = {
  total: number;
  done: number;
  running: number;
  blocked: number;
  open: number;
};

export function summarizePlan(tasks: readonly Task[]): Plan {
  // A task is blocked when something it named in `after` has not finished. Resolving that here
  // rather than trusting a flag means a dependency completing is reflected the moment it lands,
  // and it is why the board is read whole rather than counted row by row.
  const unfinished = new Set(tasks.filter((t) => !t.done).map((t) => t.id));
  let done = 0;
  let running = 0;
  let blocked = 0;
  let open = 0;
  for (const t of tasks) {
    if (t.done) done++;
    else if (t.claimedBy !== null) running++;
    else if (t.blockedBy.some((id) => unfinished.has(id))) blocked++;
    else open++;
  }
  return { total: tasks.length, done, running, blocked, open };
}

// One line of plain English for the plan, or null when there is no plan to speak of.
//
// Null rather than "0 steps" because an empty board is the normal state before the first
// instruction and for any instruction small enough that the seat just did it. A bar that reports
// zero of nothing every time it is idle trains people to stop reading it.
export function planLine(plan: Plan): string | null {
  if (plan.total === 0) return null;
  const parts: string[] = [];
  if (plan.running > 0) parts.push(`${plan.running} in progress`);
  if (plan.blocked > 0) parts.push(`${plan.blocked} waiting on another`);
  if (plan.open > 0) parts.push(`${plan.open} not started`);
  if (plan.done > 0) parts.push(`${plan.done} done`);
  const steps = plan.total === 1 ? "1 step" : `${plan.total} steps`;
  return `${steps}: ${parts.join(", ")}`;
}

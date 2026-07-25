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

// `seat` is the id the canvas remembers, `seatAlive` is whether that seat still has a live
// terminal behind it, and `defaultAgentId` is the engine's capability-ranked pick (null when
// nothing installed can be wired to the bus).
//
// Liveness rather than canvas membership is the question, because the seat is headless: it runs a
// real CLI but never becomes a node. Asking "is it on the canvas" would have answered no for every
// seat that ever existed and stood a new one up on every single instruction. A seat whose agent has
// exited reads as no seat, which is the normal aftermath of it finishing or crashing, so the bar
// quietly stands a new one up instead of typing into a process that is gone.
export function planSeat(
  seat: string | null,
  seatAlive: boolean,
  defaultAgentId: string | null,
): SeatPlan {
  if (seat !== null && seatAlive) {
    return { kind: "use", nodeId: seat };
  }
  if (defaultAgentId === null) return { kind: "unavailable" };
  return { kind: "create", agentId: defaultAgentId };
}

// Everything an agent CLI paints to redraw its own screen, which is noise once the output is being
// read as a conversation rather than run as a terminal.
//
// Three families, because they terminate differently and one regex for all of them would be wrong
// on at least one: CSI (colour, cursor moves, line clears), OSC (window titles, hyperlinks, ended
// by BEL or ST), and the short two-byte escapes like charset selection.
const CSI = /\x1b\[[0-9;?]*[ -/]*[@-~]/g;
const OSC = /\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)/g;
const SHORT_ESC = /\x1b[()][AB0-2]|\x1b[=>78MDEHc]/g;

// How much of the conversation the command center keeps. The tail is what a person is reading; the
// head is what they already read and scrolled past.
export const TRANSCRIPT_MAX = 12000;

/// Raw PTY bytes as something readable in the command center.
///
/// This deliberately does not try to reconstruct a chat UI out of a TUI. An agent CLI paints boxes,
/// moves the cursor and repaints lines in place, and faithfully replaying that into a scrolling
/// pane would need a terminal emulator, which is exactly the thing the command center exists to not
/// make the user read. What it does instead is take the escape sequences out and leave the words
/// in, which is the part a person is actually following.
///
/// The carriage-return handling is the one piece of emulation kept, because without it every
/// progress spinner and re-drawn line arrives as a run of duplicates: a CR means "go back to the
/// start of this line", so whatever was on that line before it is what got overwritten.
export function readableTranscript(raw: string): string {
  const clean = raw
    .replace(OSC, "")
    .replace(CSI, "")
    .replace(SHORT_ESC, "")
    // A line rewritten in place keeps only its final state, the same as it would look on screen.
    .split("\n")
    .map((line) => {
      const parts = line.split("\r");
      return parts[parts.length - 1] ?? "";
    })
    .join("\n")
    // Three or more blank lines is a repaint gap, not paragraphing.
    .replace(/\n{3,}/g, "\n\n");
  return clean.length > TRANSCRIPT_MAX
    ? clean.slice(clean.length - TRANSCRIPT_MAX)
    : clean;
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

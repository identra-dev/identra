// The decisions the command bar has to make before it can send anything, kept out of the component
// so they can be read and tested on their own. The component does the IO; this says what the IO
// should be.

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

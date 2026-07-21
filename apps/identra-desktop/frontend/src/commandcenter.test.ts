import { test, expect } from "bun:test";
import { planSeat, composeDispatch } from "./commandcenter";

test("the bar talks to the seat that exists, and stands one up when it does not", () => {
  // The ordinary case: a seat is remembered and its node is still there.
  expect(planSeat("n1", ["n1", "n2"], "codex")).toEqual({
    kind: "use",
    nodeId: "n1",
  });

  // Nothing has been assigned yet, so the first instruction stands the seat up with the engine's
  // capability-ranked pick rather than asking the user to go and make one first.
  expect(planSeat(null, [], "codex")).toEqual({
    kind: "create",
    agentId: "codex",
  });

  // A seat whose node was closed is the normal aftermath of closing that node, not a broken canvas.
  // It has to read as "no seat" or the bar would type into a node that is gone.
  expect(planSeat("closed-node", ["n2"], "codex")).toEqual({
    kind: "create",
    agentId: "codex",
  });

  // Nothing on this machine can be wired to the bus, so there is no seat to offer and the bar has
  // to say so instead of pretending the instruction went somewhere.
  expect(planSeat(null, [], null)).toEqual({ kind: "unavailable" });
  // Even with nothing installable, an existing seat still works: it is already running.
  expect(planSeat("n1", ["n1"], null)).toEqual({ kind: "use", nodeId: "n1" });
});

test("the brief rides in front of the first instruction and never again", () => {
  const first = composeDispatch("BRIEF", "ship the thing", true);
  expect(first).toBe("BRIEF\n\nship the thing\r");

  // Every later instruction goes through as the user typed it. Resending the brief would spend the
  // agent's context restating something it was told at the start of the session.
  expect(composeDispatch("BRIEF", "now the tests", false)).toBe(
    "now the tests\r",
  );

  // The carriage return is what submits the line. Without it the text sits on the agent's prompt
  // looking dispatched but doing nothing, which is the worst of both.
  expect(first.endsWith("\r")).toBe(true);
});

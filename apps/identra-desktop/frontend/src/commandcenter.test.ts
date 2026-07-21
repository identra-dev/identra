import { test, expect } from "bun:test";
import {
  planSeat,
  composeDispatch,
  summarizePlan,
  planLine,
} from "./commandcenter";
import type { Task } from "./api";

const task = (id: number, over: Partial<Task> = {}): Task => ({
  id,
  description: `step ${id}`,
  claimedBy: null,
  done: false,
  note: null,
  blockedBy: [],
  ...over,
});

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

test("the plan is counted off the board, with blocked resolved against what is finished", () => {
  const tasks = [
    task(1, { done: true }),
    task(2, { claimedBy: "node-b" }),
    // Waiting on 2, which is claimed but not finished, so this is blocked rather than available.
    task(3, { blockedBy: [2] }),
    // Waiting only on 1, which is done, so this is ready for anyone to claim.
    task(4, { blockedBy: [1] }),
    task(5),
  ];
  expect(summarizePlan(tasks)).toEqual({
    total: 5,
    done: 1,
    running: 1,
    blocked: 1,
    open: 2,
  });

  // A finished dependency must stop counting as a block the moment it lands, which is the whole
  // reason this is resolved against the board rather than read off a stored flag.
  expect(
    summarizePlan([task(1, { done: true }), task(2, { blockedBy: [1] })]),
  ).toEqual({ total: 2, done: 1, running: 0, blocked: 0, open: 1 });
});

test("an empty board says nothing rather than reporting zero of nothing", () => {
  // The state before the first instruction, and after any instruction small enough that the seat
  // just did it rather than splitting it. Reporting "0 steps" here trains people to stop reading.
  expect(planLine(summarizePlan([]))).toBe(null);

  expect(planLine(summarizePlan([task(1)]))).toBe("1 step: 1 not started");
  expect(
    planLine(
      summarizePlan([task(1, { done: true }), task(2, { claimedBy: "b" })]),
    ),
  ).toBe("2 steps: 1 in progress, 1 done");
});

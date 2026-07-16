import { test, expect } from "bun:test";
import { pastSnapshot } from "./reattach";

test("only live chunks past the snapshot seq are applied", () => {
  expect(pastSnapshot(5, 4)).toBe(true); // newer than snapshot -> write it
  expect(pastSnapshot(4, 4)).toBe(false); // already in the snapshot -> skip
  expect(pastSnapshot(1, 4)).toBe(false); // stale -> skip
});

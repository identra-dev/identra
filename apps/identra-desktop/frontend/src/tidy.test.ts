import { test, expect } from "bun:test";
import { tidyPositions, type Placeable } from "./tidy";

const node = (id: string, x: number, y: number): Placeable => ({
  id,
  position: { x, y },
  width: 480,
  height: 320,
});

test("tidy straightens the board without scrambling it", () => {
  // Deliberately out of order in the array, and overlapping, which is what the canvas actually
  // looks like after the command center has spawned a few helpers under one parent.
  const placed = tidyPositions([
    node("bottom-left", 10, 900),
    node("top-right", 700, 20),
    node("top-left", 20, 30),
    node("bottom-right", 690, 910),
  ]);

  // Reading order is preserved: whatever was up and to the left is still up and to the left.
  expect(placed.map((p) => p.id)).toEqual([
    "top-left",
    "top-right",
    "bottom-left",
    "bottom-right",
  ]);

  // Two columns for four nodes, so it comes out square rather than as one long strip.
  expect(placed[0]).toEqual({ id: "top-left", x: 0, y: 0 });
  expect(placed[1]).toEqual({ id: "top-right", x: 528, y: 0 });
  expect(placed[2]).toEqual({ id: "bottom-left", x: 0, y: 368 });
  expect(placed[3]).toEqual({ id: "bottom-right", x: 528, y: 368 });

  // Nothing overlaps: every cell is the widest node plus a gap, so wires stay readable.
  const xs = new Set(placed.map((p) => p.x));
  expect(Math.min(...xs)).toBe(0);
});

test("tidy handles the edges rather than dividing by zero on them", () => {
  expect(tidyPositions([])).toEqual([]);
  expect(tidyPositions([node("only", 500, 500)])).toEqual([
    { id: "only", x: 0, y: 0 },
  ]);
});

test("tidy lays out from an origin, so it can land in view rather than at the origin", () => {
  // The canvas tidies into what the user is currently looking at. Always laying out from 0,0 would
  // teleport the board off screen on a canvas that has been panned.
  expect(tidyPositions([node("a", 0, 0)], { x: 100, y: 200 })).toEqual([
    { id: "a", x: 100, y: 200 },
  ]);
});

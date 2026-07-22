import { expect, test } from "bun:test";
import { previewLayout } from "./preview";
import type { CanvasNode } from "./api";

const node = (id: string, x: number, y: number): CanvasNode => ({
  id,
  kind: "codex",
  x,
  y,
  width: 480,
  height: 320,
  title: id,
  cwd: null,
  locked: false,
});

test("an empty board previews as nothing, not as a crash or a dot", () => {
  expect(previewLayout([], 96, 60)).toEqual([]);
});

test("everything lands inside the tile, whatever the board's own coordinates", () => {
  // A board that has been panned far from the origin and spread wide. The thumbnail must not
  // care where the user happened to build.
  const rects = previewLayout(
    [node("a", -4000, 2000), node("b", 3000, 2400), node("c", -500, 5000)],
    96,
    60,
  );
  for (const r of rects) {
    expect(r.x).toBeGreaterThanOrEqual(0);
    expect(r.y).toBeGreaterThanOrEqual(0);
    expect(r.x + r.w).toBeLessThanOrEqual(96);
    expect(r.y + r.h).toBeLessThanOrEqual(60);
  }
});

test("the arrangement survives the shrink", () => {
  // b sits right of and below a on the board, so it must in the thumbnail too, by the same
  // ordering even though not the same distance.
  const [a, b] = previewLayout([node("a", 0, 0), node("b", 900, 500)], 96, 60);
  expect(a!.x).toBeLessThan(b!.x);
  expect(a!.y).toBeLessThan(b!.y);
  // Uniform scale: a node's aspect ratio is its own, not the tile's.
  expect(a!.w / a!.h).toBeCloseTo(480 / 320, 5);
});

test("one node reads as a small shape, not a screenshot", () => {
  const [only] = previewLayout([node("a", 0, 0)], 96, 60);
  // The scale cap is what keeps it small; without it a lone node fits the tile edge to edge and
  // the thumbnail stops saying "this board has one node on it".
  expect(only!.w).toBeLessThan(96 * 0.75);
});

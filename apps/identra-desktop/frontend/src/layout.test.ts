import { test, expect } from "bun:test";
import {
  clearNode,
  closeLeaf,
  leaf,
  leaves,
  setNode,
  splitLeaf,
  stepLeaf,
  type Pane,
} from "./layout";

const ids = (p: Pane) => leaves(p).map((l) => l.id);

test("splitting keeps what you were looking at and leaves room beside it", () => {
  const one = setNode(leaf("a"), "a", "claude");
  const two = splitLeaf(one, "a", "b", "row");

  expect(ids(two)).toEqual(["a", "b"]);
  expect(leaves(two)[0]!.nodeId).toBe("claude");
  // Empty, not a second view of the same agent. Splitting is making room for the next thing.
  expect(leaves(two)[1]!.nodeId).toBe(null);
});

test("splitting a leaf nested inside a split only touches that leaf", () => {
  let tree: Pane = splitLeaf(leaf("a"), "a", "b", "row");
  tree = splitLeaf(tree, "b", "c", "col");

  expect(ids(tree)).toEqual(["a", "b", "c"]);
});

test("closing a pane collapses its parent into whatever was beside it", () => {
  let tree: Pane = splitLeaf(leaf("a"), "a", "b", "row");
  tree = splitLeaf(tree, "b", "c", "col");

  const gone = closeLeaf(tree, "b");
  expect(ids(gone)).toEqual(["a", "c"]);

  // And the survivor is a plain leaf again once it is on its own, rather than a split with one
  // half missing, which would draw as a pane taking up half the column for no reason.
  expect(closeLeaf(closeLeaf(gone, "c"), "a").kind).toBe("leaf");
});

test("closing the last pane leaves it alone", () => {
  // A centre column with no panes has nowhere to put the next agent and no visible way back.
  const only = leaf("a");
  expect(closeLeaf(only, "a")).toEqual(only);
});

test("closing a node blanks every pane showing it, and no others", () => {
  let tree: Pane = splitLeaf(leaf("a"), "a", "b", "row");
  tree = setNode(tree, "a", "claude");
  tree = setNode(tree, "b", "codex");

  const after = leaves(clearNode(tree, "claude"));
  expect(after[0]!.nodeId).toBe(null);
  expect(after[1]!.nodeId).toBe("codex");
});

test("stepping between panes wraps at both ends", () => {
  const tree = splitLeaf(leaf("a"), "a", "b", "row");

  expect(stepLeaf(tree, "a", 1)).toBe("b");
  // With two panes, next and previous are the same gesture. A key that does nothing here reads as
  // a key that is broken.
  expect(stepLeaf(tree, "b", 1)).toBe("a");
  expect(stepLeaf(tree, "a", -1)).toBe("b");
});

test("stepping from a pane that is gone lands somewhere real", () => {
  const tree = splitLeaf(leaf("a"), "a", "b", "row");
  // The focused pane was just closed; the next keystroke must not walk off the tree.
  expect(stepLeaf(tree, "closed", 1)).toBe("a");
});

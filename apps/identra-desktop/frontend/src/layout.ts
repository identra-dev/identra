// The shape of the centre column: a tree of panes, each leaf showing one node or nothing.
//
// Pure, and separate from the component that draws it, for the same reason `tidy` was: the
// judgement here is "which leaf is where after this operation", and that is worth being able to
// check without mounting a terminal.
//
// This tree is deliberately not persisted. The canvas saved positions because positions were the
// product; a split ratio is not, and the users who asked for this shell asked for it by saying the
// arrangement was the part they did not want. What persists is which agents exist, which is the
// node list, and which of them may talk to each other, which is the edge list. Where their panes
// sat this session is not a fact about the project.

export type Leaf = { kind: "leaf"; id: string; nodeId: string | null };
export type Split = {
  kind: "split";
  id: string;
  // How the two halves sit next to each other. "row" is side by side, "col" is stacked.
  dir: "row" | "col";
  a: Pane;
  b: Pane;
};
export type Pane = Leaf | Split;

export const leaf = (id: string, nodeId: string | null = null): Leaf => ({
  kind: "leaf",
  id,
  nodeId,
});

/// Every leaf, left to right and top to bottom. This is the order the next/previous pane keys walk,
/// which is why it is reading order and not insertion order: the key moves you to the pane beside
/// the one you are in, and "beside" is a thing you see rather than a thing the tree remembers.
export function leaves(pane: Pane): Leaf[] {
  return pane.kind === "leaf" ? [pane] : [...leaves(pane.a), ...leaves(pane.b)];
}

/// Split `leafId` in two, keeping what it shows on the near side and leaving the new side empty.
///
/// The new leaf is empty rather than a copy. Two panes onto one agent is a thing the app supports —
/// one pty, several views — but it is never what someone means by "split": they are making room for
/// the next thing, and starting with the same agent twice makes them close one before they begin.
export function splitLeaf(
  pane: Pane,
  leafId: string,
  newLeafId: string,
  dir: "row" | "col",
): Pane {
  if (pane.kind === "leaf") {
    if (pane.id !== leafId) return pane;
    return {
      kind: "split",
      id: `split-${newLeafId}`,
      dir,
      a: pane,
      b: leaf(newLeafId),
    };
  }
  return {
    ...pane,
    a: splitLeaf(pane.a, leafId, newLeafId, dir),
    b: splitLeaf(pane.b, leafId, newLeafId, dir),
  };
}

/// Remove `leafId`, collapsing its parent split into whatever was beside it.
///
/// Closing the last pane returns the tree untouched. A centre column with no panes at all is not a
/// state worth having: there would be nothing to put the next agent into and no visible way back.
export function closeLeaf(pane: Pane, leafId: string): Pane {
  if (pane.kind === "leaf") return pane;
  if (pane.a.kind === "leaf" && pane.a.id === leafId) return pane.b;
  if (pane.b.kind === "leaf" && pane.b.id === leafId) return pane.a;
  return {
    ...pane,
    a: closeLeaf(pane.a, leafId),
    b: closeLeaf(pane.b, leafId),
  };
}

/// Show `nodeId` in `leafId`. This is what clicking a tab does.
export function setNode(pane: Pane, leafId: string, nodeId: string): Pane {
  if (pane.kind === "leaf")
    return pane.id === leafId ? { ...pane, nodeId } : pane;
  return {
    ...pane,
    a: setNode(pane.a, leafId, nodeId),
    b: setNode(pane.b, leafId, nodeId),
  };
}

/// Blank every leaf showing `nodeId`. Called when a node is closed for good: the pane stays, the
/// thing it was showing does not. Leaving a dead id in place would mount a terminal onto a pty that
/// no longer exists, which draws as an empty black box with no explanation.
export function clearNode(pane: Pane, nodeId: string): Pane {
  if (pane.kind === "leaf")
    return pane.nodeId === nodeId ? { ...pane, nodeId: null } : pane;
  return {
    ...pane,
    a: clearNode(pane.a, nodeId),
    b: clearNode(pane.b, nodeId),
  };
}

/// The leaf `delta` steps from `fromId` in reading order, wrapping at both ends.
///
/// Wrapping rather than stopping: with two panes, next and previous are the same gesture, and a key
/// that does nothing at the end of a list of two reads as a key that is broken.
export function stepLeaf(pane: Pane, fromId: string, delta: number): string {
  const all = leaves(pane);
  const at = all.findIndex((l) => l.id === fromId);
  if (at === -1) return all[0]?.id ?? fromId;
  const next = (at + delta + all.length) % all.length;
  return all[next]!.id;
}

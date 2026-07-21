// Straightening a canvas that has drifted.
//
// This matters more than it used to. When every node was placed by hand it stayed roughly where it
// was put, but the command center spawns helpers on the user's behalf and stacks them under their
// parent, so a canvas that has run a few instructions ends up with nodes sitting on top of each
// other. Tidy is the way back without dragging each one.

export type Placeable = {
  id: string;
  position: { x: number; y: number };
  width: number;
  height: number;
};

export type Placement = { id: string; x: number; y: number };

// Gap between cells. Wide enough that the wires between nodes are readable rather than running
// straight from one border into the next.
const GAP = 48;

// Lay the nodes out on a grid, keeping the order they already read in.
//
// Sorting by position first is what makes this feel like straightening rather than shuffling: a
// node that was roughly top-left stays roughly top-left, so the user's own mental map of the canvas
// survives the operation. Sorting by id instead would be simpler and would scramble the board every
// time, which is the kind of tidy nobody presses twice.
//
// One uniform cell, sized to the largest node, rather than packing tightly. Tight packing with
// mixed node sizes gives ragged columns that look like a bug, and the whole point of the button is
// that the result looks deliberate.
export function tidyPositions(
  nodes: readonly Placeable[],
  origin: { x: number; y: number } = { x: 0, y: 0 },
): Placement[] {
  if (nodes.length === 0) return [];

  // Rows first, then columns within a row. The row band is the tallest node, so two nodes whose
  // tops differ by a few pixels are treated as the same row rather than as a column of two.
  const band = Math.max(...nodes.map((n) => n.height));
  const ordered = [...nodes].sort((a, b) => {
    const rowA = Math.floor(a.position.y / band);
    const rowB = Math.floor(b.position.y / band);
    if (rowA !== rowB) return rowA - rowB;
    return a.position.x - b.position.x;
  });

  const cellW = Math.max(...nodes.map((n) => n.width)) + GAP;
  const cellH = band + GAP;
  // A square-ish grid. Growing in one direction only turns twelve nodes into a column you cannot
  // see the end of, and the canvas zooms to fit better when it is not one long strip.
  const cols = Math.ceil(Math.sqrt(ordered.length));

  return ordered.map((n, i) => ({
    id: n.id,
    x: origin.x + (i % cols) * cellW,
    y: origin.y + Math.floor(i / cols) * cellH,
  }));
}

// The geometry behind a board thumbnail: fit every node into a small tile, keeping their real
// arrangement. Pure, so the judgement (what fits where) is testable without drawing anything.
import type { CanvasNode } from "./api";

export type PreviewRect = {
  id: string;
  x: number;
  y: number;
  w: number;
  h: number;
  kind: string;
};

// Canvas units of breathing room around the content, before scaling. Without it a lone node's
// rect touches the tile edge and reads as cropped rather than small.
const PAD = 60;

/// Scale and translate the board's nodes into a tile of `tileW` by `tileH`.
///
/// The whole content box is fitted, uniformly, and centered: the thumbnail is the shape of the
/// user's arrangement, not a viewport into it. An empty board returns nothing and the tile shows
/// only the wallpaper, which is honest: that is what an empty board looks like.
export function previewLayout(
  nodes: CanvasNode[],
  tileW: number,
  tileH: number,
): PreviewRect[] {
  if (nodes.length === 0) return [];
  const minX = Math.min(...nodes.map((n) => n.x)) - PAD;
  const minY = Math.min(...nodes.map((n) => n.y)) - PAD;
  const maxX = Math.max(...nodes.map((n) => n.x + n.width)) + PAD;
  const maxY = Math.max(...nodes.map((n) => n.y + n.height)) + PAD;
  // One scale for both axes, never more than fits. The cap keeps a single node from filling the
  // tile like a screenshot: a thumbnail that is one big rectangle says nothing.
  const scale = Math.min(tileW / (maxX - minX), tileH / (maxY - minY), 0.14);
  // Center what the scale left over, so a wide board sits in the middle of the tile rather than
  // hugging a corner.
  const dx = (tileW - (maxX - minX) * scale) / 2;
  const dy = (tileH - (maxY - minY) * scale) / 2;
  return nodes.map((n) => ({
    id: n.id,
    x: (n.x - minX) * scale + dx,
    y: (n.y - minY) * scale + dy,
    w: n.width * scale,
    h: n.height * scale,
    kind: n.kind,
  }));
}

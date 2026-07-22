// A board in miniature: the workspace's wallpaper with its nodes drawn as small shapes, edges
// and all. Drawn from the canvas data, never captured, so it costs one layout pass and works for
// a workspace that has never been on screen.
//
// The layout is computed at the given size and the svg carries a viewBox, so CSS can stretch the
// same drawing: the picker row keeps it at postage-stamp size, the home grid shows it as a card.
import { convertFileSrc } from "@tauri-apps/api/core";
import type { Canvas } from "./api";
import { auraFor } from "./icons";
import { previewLayout } from "./preview";
import { backgroundCss, needsScrim } from "./wallpaper";

type Props = {
  canvas: Canvas;
  width?: number;
  height?: number;
  className?: string;
};

export default function BoardPreview({
  canvas,
  width = 96,
  height = 60,
  className = "identra-preview",
}: Props) {
  const rects = previewLayout(canvas.nodes, width, height);
  const byId = new Map(rects.map((r) => [r.id, r]));
  return (
    <span
      className={className}
      data-scrim={needsScrim(canvas.wallpaper) || undefined}
      style={{ background: backgroundCss(canvas.wallpaper, convertFileSrc) }}
      aria-hidden="true"
    >
      <svg viewBox={`0 0 ${width} ${height}`}>
        {/* Edges first so the nodes sit on top of their own wires, same as the real canvas. An
            edge whose end is gone (a node deleted mid-save) is just not drawn. */}
        {canvas.edges.map((e) => {
          const a = byId.get(e.source);
          const b = byId.get(e.target);
          if (!a || !b) return null;
          return (
            <line
              key={e.id}
              x1={a.x + a.w / 2}
              y1={a.y + a.h / 2}
              x2={b.x + b.w / 2}
              y2={b.y + b.h / 2}
              stroke="#8f8f8f"
              strokeWidth={1}
            />
          );
        })}
        {rects.map((r) => (
          <rect
            key={r.id}
            x={r.x}
            y={r.y}
            width={r.w}
            height={r.h}
            rx={2}
            fill="#2c2c2c"
            stroke={auraFor(r.kind)}
            strokeWidth={1}
          />
        ))}
      </svg>
    </span>
  );
}

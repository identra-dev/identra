// App identity for the dock tiles and node headers: each agent's own mark, keyed by the id the
// engine reports. These are the real product marks, drawn as inline SVG paths so the dock reads
// like a desktop of actual apps. Inline beats shipping image files: one flat fill plus one path
// scales cleanly at any tile size, and restyling here never recompiles the engine.
//
// The marks identify the CLI each node runs, which is what a launcher is for. An agent with no
// published mark falls back to a lettermark rather than a wrong logo.

type Mark = {
  /// Tile fill behind the mark.
  tile: string;
  /// The colour this agent glows while working. Defaults to the tile where the tile reads on the
  /// dark canvas; set explicitly where the tile is white or near black and would not.
  aura?: string;
  /// The mark itself, and the letter's color in the fallback.
  ink: string;
  /// 24x24 path. Absent means "no published mark", so `letter` is drawn instead.
  path?: string;
  letter?: string;
};

const OPENAI =
  "M9.205 8.658v-2.26c0-.19.072-.333.238-.428l4.543-2.616c.619-.357 1.356-.523 2.117-.523 2.854 0 4.662 2.212 4.662 4.566 0 .167 0 .357-.024.547l-4.71-2.759a.797.797 0 00-.856 0l-5.97 3.473zm10.609 8.8V12.06c0-.333-.143-.57-.429-.737l-5.97-3.473 1.95-1.118a.433.433 0 01.476 0l4.543 2.617c1.309.76 2.189 2.378 2.189 3.948 0 1.808-1.07 3.473-2.76 4.163zM7.802 12.703l-1.95-1.142c-.167-.095-.239-.238-.239-.428V5.899c0-2.545 1.95-4.472 4.591-4.472 1 0 1.927.333 2.712.928L8.23 5.067c-.285.166-.428.404-.428.737v6.898zM12 15.128l-2.795-1.57v-3.33L12 8.658l2.795 1.57v3.33L12 15.128zm1.796 7.23c-1 0-1.927-.332-2.712-.927l4.686-2.712c.285-.166.428-.404.428-.737v-6.898l1.974 1.142c.167.095.238.238.238.428v5.233c0 2.545-1.974 4.472-4.614 4.472zm-5.637-5.303l-4.544-2.617c-1.308-.761-2.188-2.378-2.188-3.948A4.482 4.482 0 014.21 6.327v5.423c0 .333.143.571.428.738l5.947 3.449-1.95 1.118a.432.432 0 01-.476 0zm-.262 3.9c-2.688 0-4.662-2.021-4.662-4.519 0-.19.024-.38.047-.57l4.686 2.71c.286.167.571.167.856 0l5.97-3.448v2.26c0 .19-.07.333-.237.428l-4.543 2.616c-.619.357-1.356.523-2.117.523zm5.899 2.83a5.947 5.947 0 005.827-4.756C22.287 18.339 24 15.84 24 13.296c0-1.665-.713-3.282-1.998-4.448.119-.5.19-.999.19-1.498 0-3.401-2.759-5.947-5.946-5.947-.642 0-1.26.095-1.88.31A5.962 5.962 0 0010.205 0a5.947 5.947 0 00-5.827 4.757C1.713 5.447 0 7.945 0 10.49c0 1.666.713 3.283 1.998 4.448-.119.5-.19 1-.19 1.499 0 3.401 2.759 5.946 5.946 5.946.642 0 1.26-.095 1.88-.309a5.96 5.96 0 004.162 1.713z";

const CLAUDE_CODE =
  "M21 10.5h3v3h-3v3h-1.5v3H18v-3h-1.5v3H15v-3H9v3H7.5v-3H6v3H4.5v-3H3v-3H0v-3h3v-6h18Zm-15 0h1.5v-3H6Zm10.5 0H18v-3h-1.5z";

const GEMINI =
  "M11.04 19.32Q12 21.51 12 24q0-2.49.93-4.68.96-2.19 2.58-3.81t3.81-2.55Q21.51 12 24 12q-2.49 0-4.68-.93a12.3 12.3 0 0 1-3.81-2.58 12.3 12.3 0 0 1-2.58-3.81Q12 2.49 12 0q0 2.49-.96 4.68-.93 2.19-2.55 3.81a12.3 12.3 0 0 1-3.81 2.58Q2.49 12 0 12q2.49 0 4.68.96 2.19.93 3.81 2.55t2.55 3.81";

const OPENCODE = "M22 24H2V0h20zM17 4.8H7v14.4h10z";

const CURSOR =
  "M11.503.131 1.891 5.678a.84.84 0 0 0-.42.726v11.188c0 .3.162.575.42.724l9.609 5.55a1 1 0 0 0 .998 0l9.61-5.55a.84.84 0 0 0 .42-.724V6.404a.84.84 0 0 0-.42-.726L12.497.131a1.01 1.01 0 0 0-.996 0M2.657 6.338h18.55c.263 0 .43.287.297.515L12.23 22.918c-.062.107-.229.064-.229-.06V12.335a.59.59 0 0 0-.295-.51l-9.11-5.257c-.109-.063-.064-.23.061-.23";

const AMP =
  "M12 0c6.628 0 12 5.373 12 12s-5.372 12-12 12C5.373 24 0 18.627 0 12S5.373 0 12 0zm-.92 19.278l5.034-8.377a.444.444 0 00.097-.268.455.455 0 00-.455-.455l-2.851.004.924-5.468-.927-.003-5.018 8.367s-.1.183-.1.291c0 .251.204.455.455.455l2.831-.004-.901 5.458z";

// A globe for the browser node: outline, equator, and the two meridians that read as a globe at
// tile size. Mine, because the browser node is Identra's own surface, not a vendor's app.
const GLOBE =
  "M12 1.5a10.5 10.5 0 100 21 10.5 10.5 0 000-21zm0 1.6c1.42 0 2.9 2.02 3.5 5.15H8.5C9.1 5.12 10.58 3.1 12 3.1zM7.1 8.25H3.9A8.94 8.94 0 018.6 3.9c-.66 1.16-1.17 2.65-1.5 4.35zm-3.9 1.6h3.65a22 22 0 000 4.3H3.2a8.9 8.9 0 010-4.3zm4 4.3a20 20 0 010-4.3h9.6a20 20 0 010 4.3zm-.1 1.6c.33 1.7.84 3.19 1.5 4.35a8.94 8.94 0 01-4.7-4.35zm1.4 0h7c-.6 3.13-2.08 5.15-3.5 5.15s-2.9-2.02-3.5-5.15zm8.4 0h3.2a8.94 8.94 0 01-4.7 4.35c.66-1.16 1.17-2.65 1.5-4.35zm3.9-1.6h-3.65a22 22 0 000-4.3h3.65a8.9 8.9 0 010 4.3zm-.7-5.9h-3.2c-.33-1.7-.84-3.19-1.5-4.35a8.94 8.94 0 014.7 4.35z";

const MARKS: Record<string, Mark> = {
  codex: { tile: "#0d0d0d", ink: "#ffffff", path: OPENAI, aura: "#10a37f" },
  claude: {
    tile: "#d97757",
    ink: "#ffffff",
    path: CLAUDE_CODE,
    aura: "#d97757",
  },
  gemini: { tile: "#ffffff", ink: "#8e75b2", path: GEMINI, aura: "#8e75b2" },
  opencode: {
    tile: "#f6f5f4",
    ink: "#000000",
    path: OPENCODE,
    aura: "#22d3ee",
  },
  "cursor-agent": {
    tile: "#f6f5f4",
    ink: "#000000",
    path: CURSOR,
    aura: "#f6f5f4",
  },
  amp: { tile: "#005af0", ink: "#ffffff", path: AMP, aura: "#005af0" },
  // No published mark I can use, so these carry a lettermark rather than an invented logo.
  aider: { tile: "#17b217", ink: "#ffffff", letter: "A" },
  goose: { tile: "#b5835a", ink: "#ffffff", letter: "G" },
  browser: { tile: "#3584e4", ink: "#ffffff", path: GLOBE },
  // The dev server node: Identra's own surface, running the project's own command. The prompt
  // glyph says "this is a process", and green is the terminal-green everyone reads as running.
  dev: { tile: "#26a269", ink: "#ffffff", letter: ">", aura: "#26a269" },
};

// A saved canvas can name an agent this build has no mark for. Paint it neutral with its initial
// rather than leaving a blank tile.
const fallback = (kind: string): Mark => ({
  tile: "#5e5c64",
  ink: "#ffffff",
  letter: (kind[0] ?? "?").toUpperCase(),
});

/// The colour a node glows while its agent is working. It is the agent's own brand colour, so you
/// can tell which one is busy from the corner of your eye without reading anything.
export const auraFor = (kind: string): string =>
  (MARKS[kind] ?? fallback(kind)).aura ?? "#e95420";

export function AgentIcon({
  kind,
  className,
}: {
  kind: string;
  className?: string;
}) {
  const mark = MARKS[kind] ?? fallback(kind);
  return (
    <span
      className={className}
      style={{ background: mark.tile, color: mark.ink }}
    >
      {mark.path ? (
        <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
          <path d={mark.path} />
        </svg>
      ) : (
        mark.letter
      )}
    </span>
  );
}

// App identity for the dock tiles and node headers: one flat Yaru fill plus a glyph per agent,
// keyed by the same id the engine reports. Flat fills, no gradients, no image assets — the glyph
// carries the brand and reads at 20px. Restyling here never recompiles the engine.
export type AgentIcon = { tile: string; glyph: string };

const ICONS: Record<string, AgentIcon> = {
  codex: { tile: "#0d0d0d", glyph: ">_" },
  claude: { tile: "#d97757", glyph: "✳" }, // sunburst
  gemini: { tile: "#4285f4", glyph: "✦" }, // four-point sparkle
  opencode: { tile: "#211e1e", glyph: "oc" },
  aider: { tile: "#17b217", glyph: "ai" },
  goose: { tile: "#b5835a", glyph: "\u{1F9A2}" }, // swan/goose
  amp: { tile: "#0b0b0b", glyph: "A" },
  "cursor-agent": { tile: "#000000", glyph: "➜" }, // pointer
  browser: { tile: "#3584e4", glyph: "\u{1F310}" }, // globe
};

// A saved canvas can name an agent this build has no tile for; paint it neutral rather than blank.
const FALLBACK: AgentIcon = { tile: "#5e5c64", glyph: "▣" };

export const iconFor = (id: string): AgentIcon => ICONS[id] ?? FALLBACK;

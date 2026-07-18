import type { AgentInfo } from "./api";

// First run with no agent CLI on this machine. Without this the dock is a row of disabled buttons
// and the empty-canvas hint tells you to "pick an agent" you have no way to pick. This says what to
// install and then gets out of the way the moment one is found.

// The agents Identra fronts. Codex leads because it is the one with an exact install page to point
// at; the others are named so you know what else runs, without a shell command that would drift by
// OS and version and be wrong more often than not.
const FRONTED = ["codex", "claude", "gemini", "opencode"];

type Props = {
  agents: AgentInfo[];
  onRecheck: () => void;
};

// "a and b", "a, b, and c": read the list back as a sentence rather than a comma dump.
function joinNames(names: string[]): string {
  if (names.length <= 1) return names[0] ?? "";
  if (names.length === 2) return `${names[0]} and ${names[1]}`;
  return `${names.slice(0, -1).join(", ")}, and ${names[names.length - 1]}`;
}

export default function Onboarding({ agents, onRecheck }: Props) {
  const others = agents
    .filter((a) => FRONTED.includes(a.id) && a.id !== "codex")
    .map((a) => a.name);
  return (
    <div
      className="identra-onboard"
      role="dialog"
      aria-label="Install a coding agent"
    >
      <h1 className="identra-onboard__title">No coding agent found</h1>
      <p className="identra-onboard__lead">
        Identra runs coding agents on your machine, it does not ship one.
        Install one and it shows up in the dock below.
      </p>
      <div className="identra-onboard__agent">
        <span className="identra-onboard__name">Codex</span>
        <span className="identra-onboard__where">github.com/openai/codex</span>
      </div>
      {others.length > 0 && (
        <p className="identra-onboard__more">
          Identra also runs {joinNames(others)}. Install any and it appears
          here.
        </p>
      )}
      <button className="identra-onboard__btn" onClick={onRecheck}>
        Check again
      </button>
    </div>
  );
}

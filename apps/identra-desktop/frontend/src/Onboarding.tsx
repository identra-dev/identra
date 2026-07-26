import { useState } from "react";
import logo from "./assets/identra.png";
import type { AgentInfo } from "./api";

// First run with no agent CLI on this machine. Without this the dock is a row of disabled buttons
// and the empty-canvas hint tells you to "pick an agent" you have no way to pick. This says what to
// install and then gets out of the way the moment one is found.

// The agents Identra fronts, and which one this screen names first.
//
// It led with codex, on the reasoning that codex had the tidiest install page to point at. That is
// a real consideration and it is the wrong one, because the agent someone installs here is the
// agent they judge Identra by. Identra's whole pitch is that a project's memory is waiting for an
// agent the moment it connects, and that arrives over the MCP handshake's instructions field —
// which claude surfaces and codex does not. Someone who takes this screen's advice therefore
// installs the one agent that quietly turns the headline feature off, then forms their opinion of
// the product without ever seeing it work.
//
// So: lead with the one that shows what this is for. The rest are named because they do run here
// and someone who already has one should not be told to install anything.
//
// A page rather than a shell command, deliberately: an install line drifts by OS, by version and
// by package manager, and a wrong one on the first screen is worse than no line at all.
const LEAD = { id: "claude", name: "Claude Code", where: "claude.com/claude-code" };
const FRONTED = ["claude", "codex", "gemini", "opencode"];

type Props = {
  agents: AgentInfo[];
  onRecheck: () => Promise<void>;
};

// "a and b", "a, b, and c": read the list back as a sentence rather than a comma dump.
function joinNames(names: string[]): string {
  if (names.length <= 1) return names[0] ?? "";
  if (names.length === 2) return `${names[0]} and ${names[1]}`;
  return `${names.slice(0, -1).join(", ")}, and ${names[names.length - 1]}`;
}

export default function Onboarding({ agents, onRecheck }: Props) {
  const others = agents
    .filter((a) => FRONTED.includes(a.id) && a.id !== LEAD.id)
    .map((a) => a.name);
  // The probe is fast, but a button that changes nothing on screen reads as broken, and a
  // tester read it exactly that way. So the click shows its work: a checking state while the
  // probe runs, and the failure in words if it fails. Success needs no message here, because
  // the panel itself disappears the moment an agent is found.
  const [checking, setChecking] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const recheck = async () => {
    setChecking(true);
    setError(null);
    try {
      await onRecheck();
    } catch (e) {
      setError(String(e));
    } finally {
      setChecking(false);
    }
  };
  return (
    <div
      className="identra-onboard"
      role="dialog"
      aria-label="Install a coding agent"
    >
      <img className="identra-logo identra-onboard__logo" src={logo} alt="" />
      <h1 className="identra-onboard__title">No coding agent found</h1>
      <p className="identra-onboard__lead">
        Identra runs coding agents on your machine, it does not ship one.
        Install one and it shows up in the dock below.
      </p>
      <div className="identra-onboard__agent">
        <span className="identra-onboard__name">{LEAD.name}</span>
        <span className="identra-onboard__where">{LEAD.where}</span>
      </div>
      {others.length > 0 && (
        <p className="identra-onboard__more">
          Identra also runs {joinNames(others)}. Install any and it appears
          here.
        </p>
      )}
      <button
        className="identra-onboard__btn"
        disabled={checking}
        onClick={() => void recheck()}
      >
        {checking ? "Checking..." : "Check again"}
      </button>
      {error !== null && (
        <p className="identra-onboard__error" role="alert">
          The check itself failed: {error}
        </p>
      )}
    </div>
  );
}

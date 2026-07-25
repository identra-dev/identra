// One node's conversation at full window size. A second view of the same PTY, not a second
// conversation: it attaches the way a reloaded node does (snapshot, then the stream from where
// the snapshot ends) and types into the same pipe, so the canvas node behind it stays live and
// nothing is moved or paused to look at it.
import { useCallback, useEffect, useRef } from "react";
import { useAttachedTerminal } from "./attachTerminal";
import { AgentIcon } from "./icons";

type Props = {
  nodeId: string;
  title: string;
  kind: string;
  onClose: () => void;
};

export default function FocusView({ nodeId, title, kind, onClose }: Props) {
  const host = useRef<HTMLDivElement>(null);
  // Through a ref so the terminal's key handler, wired once, always calls the current one.
  const close = useRef(onClose);
  close.current = onClose;

  // Ctrl+Esc, not Esc. Plain Esc belongs to whatever runs in the terminal: the agent TUIs bind
  // it (cancel the composer, interrupt) and so does vim, so a close key of Esc would make
  // leaving this view also poke the conversation. Driving the first build proved it: the byte
  // landed in the PTY as ^[ and the view stayed open.
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape" && e.ctrlKey) onClose();
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [onClose]);

  // The same rule as the window listener, but inside the terminal, where keydown never bubbles
  // out: Ctrl+Esc closes and is swallowed, everything else reaches the agent.
  const keyHandler = useCallback((e: KeyboardEvent) => {
    if (e.type === "keydown" && e.key === "Escape" && e.ctrlKey) {
      close.current();
      return false;
    }
    return true;
  }, []);

  useAttachedTerminal(host, nodeId, {
    kind,
    fontSize: 14,
    focusOnAttach: true,
    onKeyEvent: keyHandler,
  });

  return (
    <div className="identra-focus">
      <div className="identra-focus__bar">
        <AgentIcon kind={kind} className="identra-node__icon" />
        <span className="identra-focus__title">{title || kind}</span>
        <span className="identra-focus__hint">Ctrl+Esc closes</span>
        <button
          className="identra-focus__close"
          title="Back to the canvas"
          onClick={onClose}
        >
          &times;
        </button>
      </div>
      <div className="identra-focus__term" ref={host} />
    </div>
  );
}

import { useEffect, useRef, useState, type FormEvent } from "react";

// The shortcut label, in the idiom of the machine it is drawn on. Read once: it cannot change
// while the app is open, and it is on screen at all times. Exported because the empty canvas names
// the same key, and two places spelling a shortcut is how they come to disagree.
export const MOD_LABEL = /Mac|iPhone|iPad/.test(navigator.userAgent)
  ? "⌘K"
  : "Ctrl K";

// How the last instruction went. The bar is the one place where a user gives an instruction without
// watching a terminal, so it has to answer for itself: silence after typing is indistinguishable
// from a bar that is broken.
export type DispatchState =
  | { kind: "idle" }
  | { kind: "sending"; note: string }
  | { kind: "sent"; note: string }
  | { kind: "failed"; error: string };

type Props = {
  // Resolved seat name for the label, or null when there is no seat yet. The bar still accepts an
  // instruction in that case: standing the seat up is part of dispatching the first one.
  seatName: string | null;
  state: DispatchState;
  // One line of plain English for what the seat has broken the work into, or null when the board is
  // empty. Counted off the shared board, so it is what is really happening rather than what an
  // agent said it would do.
  plan: string | null;
  // The seat has stopped and the last thing it printed reads like a question. This is the moment
  // the user is the blocker, and the bar is where the answer goes, so it has to say so.
  awaitingAnswer: boolean;
  // What the orchestrator has said so far, already stripped of escape sequences. Empty before the
  // first instruction of a session, which is the one state where the pane is not drawn at all.
  transcript: string;
  onSubmit: (instruction: string) => void;
};

/// The command center's input. One line the user types an instruction into, which goes to the
/// orchestrator seat.
///
/// It stays deliberately thin. The bar does not decide anything about the seat and holds no canvas
/// state; it collects a line, hands it up, and reports what came back. Everything about which node
/// the instruction reaches lives in App, next to the canvas state it needs.
export default function CommandBar({
  seatName,
  state,
  plan,
  awaitingAnswer,
  transcript,
  onSubmit,
}: Props) {
  const [text, setText] = useState("");
  const busy = state.kind === "sending";

  // The whole product is "say what you want done", and until now the only way to say it was to
  // find the box with a mouse. Every node on this canvas is a terminal that swallows keystrokes,
  // so there is no tab-your-way-there either: without this the primary action is the one thing on
  // screen the keyboard cannot reach.
  //
  // The listener lives here rather than in App because the bar owns the input it focuses, and it
  // is mounted exactly when the shortcut should work: on a machine with no agent installed there
  // is no bar and no seat to type at, and a shortcut that focuses nothing is worse than none.
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key !== "k" || !(e.metaKey || e.ctrlKey) || e.altKey) return;
      // Taken from the terminal too. xterm's hidden textarea has focus most of the time a node is
      // open, and Ctrl+K there is kill-to-end-of-line. Capturing on window is what gets here first;
      // stopping propagation is what actually keeps it, because xterm's own handler does not look
      // at defaultPrevented and would send the byte to the pty regardless.
      e.preventDefault();
      e.stopPropagation();
      input.current?.focus();
      input.current?.select();
    };
    window.addEventListener("keydown", key, true);
    return () => window.removeEventListener("keydown", key, true);
  }, []);

  // Follow the tail as the agent writes, the way a terminal does, but stop following the moment the
  // user scrolls up: they are reading something, and yanking them back to the bottom every time a
  // token arrives makes the pane unusable for the one thing it is for. Re-pinning when they return
  // to the bottom is what makes that reversible without a control to explain.
  const pane = useRef<HTMLPreElement>(null);
  const pinned = useRef(true);
  useEffect(() => {
    const el = pane.current;
    if (el !== null && pinned.current) el.scrollTop = el.scrollHeight;
  }, [transcript]);

  const submit = (e: FormEvent) => {
    e.preventDefault();
    const instruction = text.trim();
    if (!instruction || busy) return;
    // Clear on send rather than on success. The instruction is on its way to an agent that will
    // take a while, and leaving it in the box invites sending it twice.
    setText("");
    onSubmit(instruction);
  };

  return (
    <form
      className="identra-cmd"
      data-asking={awaitingAnswer}
      onSubmit={submit}
    >
      {awaitingAnswer && (
        // The one case where the bar interrupts. Everything else it says is progress the user can
        // read or ignore; this is the work being stopped until they answer, and the answer goes in
        // the same box, so it belongs above the input rather than below it.
        <p className="identra-cmd__asking" role="alert">
          The orchestrator has asked you something and is waiting. Its question
          is at the end of the conversation above; answer here.
        </p>
      )}
      {transcript !== "" && (
        // The conversation, in the place the instruction was typed. This is the whole point of the
        // command center being headless: the orchestrator used to land on the canvas as a node, so
        // typing here meant going somewhere else to find out what happened. Reading and replying in
        // one place is what a chat is.
        <pre
          className="identra-cmd__transcript nodrag nowheel"
          ref={pane}
          onScroll={(e) => {
            const el = e.currentTarget;
            // A small slack, because a fractional scrollHeight means an element pinned to the
            // bottom is rarely exactly at it.
            pinned.current =
              el.scrollHeight - el.scrollTop - el.clientHeight < 24;
          }}
          aria-label="What the orchestrator has said"
          aria-live="polite"
        >
          {transcript}
        </pre>
      )}
      <div className="identra-cmd__row">
        <span className="identra-cmd__label">
          {seatName === null ? "Command center" : `Command center: ${seatName}`}
        </span>
        <input
          ref={input}
          className="identra-cmd__input nodrag"
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            // Back to the canvas without sending. The same key inside a node belongs to the agent
            // TUI, which is why only the bar's own input takes it.
            if (e.key === "Escape") e.currentTarget.blur();
          }}
          disabled={busy}
          placeholder={
            seatName === null
              ? "Say what you want done, and an orchestrator starts up"
              : "Say what you want done"
          }
          aria-label="Instruction for the orchestrator"
        />
        {/* The shortcut, worn where it works. A key nobody is told about is a key nobody presses,
            and this is the one that reaches the primary action from anywhere in the app. */}
        <kbd className="identra-cmd__kbd" aria-hidden="true">
          {MOD_LABEL}
        </kbd>
        <button
          className="identra-cmd__send"
          type="submit"
          disabled={busy || text.trim() === ""}
        >
          Send
        </button>
      </div>
      {plan !== null && (
        // The plan sits under the input and stays there while the work runs. It is the answer to
        // "did anything actually come of what I typed", which a terminal full of scrolling output
        // does not give you at a glance.
        <p className="identra-cmd__plan">{plan}</p>
      )}
      {state.kind !== "idle" && (
        // One line, under the input, that says what happened to the last thing sent. It is
        // role=status rather than an alert: this is progress, not an interruption, except when it
        // failed, and a failure here is still the user's own instruction not going anywhere.
        <p
          className="identra-cmd__state"
          data-kind={state.kind}
          role={state.kind === "failed" ? "alert" : "status"}
        >
          {state.kind === "failed" ? state.error : state.note}
        </p>
      )}
    </form>
  );
}

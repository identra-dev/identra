import { useState, type FormEvent } from "react";

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
  onSubmit,
}: Props) {
  const [text, setText] = useState("");
  const busy = state.kind === "sending";

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
          The orchestrator has asked you something and is waiting. Read its
          node, then answer here.
        </p>
      )}
      <div className="identra-cmd__row">
        <span className="identra-cmd__label">
          {seatName === null ? "Command center" : `Command center: ${seatName}`}
        </span>
        <input
          className="identra-cmd__input nodrag"
          value={text}
          onChange={(e) => setText(e.target.value)}
          disabled={busy}
          placeholder={
            seatName === null
              ? "Say what you want done, and an orchestrator starts up"
              : "Say what you want done"
          }
          aria-label="Instruction for the orchestrator"
        />
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

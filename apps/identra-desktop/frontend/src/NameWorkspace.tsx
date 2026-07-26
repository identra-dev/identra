// Naming a workspace when you make one, rather than living with "untitled-workspace".
//
// The engine always accepted a title; nothing ever asked for one, so every new workspace was
// untitled-workspace, then untitled-workspace-2, and so on. That is fine for the first one and
// useless by the third: the picker shows a list of boards, and a list where every entry is the
// same word is a list you have to open one by one to read.
//
// Asked at the moment of making it, because that is when someone knows what it is for. Renaming
// later exists and nobody does it.
import { useState } from "react";

// What goes in the box before anyone types. Real-looking rather than clever: this one is a folder
// name, so the suggestion should read like something you would actually call a project.
const PLACEHOLDER = "billing rewrite";

type Props = {
  onCancel: () => void;
  // The trimmed name, or "" when someone just pressed enter on an empty box. Empty is a real
  // answer and means "you pick" — the engine falls back to its own default, which is exactly what
  // happened before this dialog existed.
  onName: (name: string) => void;
};

export default function NameWorkspace({ onCancel, onName }: Props) {
  const [typed, setTyped] = useState("");

  return (
    <div
      className="identra-ask__scrim"
      // Clicking away is a cancel, the same as Escape. A dialog with one field and a way out does
      // not need to trap anybody.
      onMouseDown={onCancel}
    >
      <div
        className="identra-ask"
        role="dialog"
        aria-label="Name this workspace"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <p className="identra-ask__lead">What is this workspace called?</p>
        <p className="identra-ask__sub">
          It names the folder Identra makes for it, and it is what you will see
          in the picker. You can rename it later.
        </p>
        <form
          className="identra-ask__row"
          onSubmit={(e) => {
            e.preventDefault();
            onName(typed.trim());
          }}
        >
          <input
            className="identra-ask__input"
            value={typed}
            onChange={(e) => setTyped(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") onCancel();
            }}
            placeholder={PLACEHOLDER}
            aria-label="Workspace name"
            maxLength={60}
            autoFocus
          />
          <button className="identra-ask__go" type="submit">
            Create
          </button>
        </form>
        <button
          className="identra-ask__skip"
          type="button"
          onClick={() => onName("")}
        >
          Skip, name it later
        </button>
      </div>
    </div>
  );
}

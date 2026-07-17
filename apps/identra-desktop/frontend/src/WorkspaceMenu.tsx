import { useEffect, useRef, useState } from "react";
import {
  workspaceCreate,
  workspaceDelete,
  workspaceList,
  workspaceRename,
  type WorkspaceMeta,
} from "./api";

// The way out of the workspace you are in. Without this the picker was a one way door: you chose at
// launch and the only way to a different workspace was to quit.
//
// Rename and delete live here rather than in the picker because this is where you are when you
// realise the name is wrong, and because delete needs the thing it is about to take to be the thing
// you are looking at.
export default function WorkspaceMenu({
  workspace,
  onOpen,
  onDeleted,
  onRenamed,
}: {
  workspace: WorkspaceMeta;
  onOpen: (w: WorkspaceMeta) => void;
  onDeleted: () => void;
  onRenamed: (w: WorkspaceMeta) => void;
}) {
  const [open, setOpen] = useState(false);
  const [all, setAll] = useState<WorkspaceMeta[]>([]);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(workspace.title);
  const [error, setError] = useState<string | null>(null);
  const root = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    void workspaceList().then(setAll, (e) => setError(String(e)));
    // Any click that is not in here closes it. A menu you have to aim at to dismiss is a menu that
    // is in your way.
    const away = (e: MouseEvent) => {
      if (!root.current?.contains(e.target as globalThis.Node)) setOpen(false);
    };
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    document.addEventListener("mousedown", away);
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("mousedown", away);
      document.removeEventListener("keydown", esc);
    };
  }, [open]);

  const commitRename = async () => {
    setEditing(false);
    if (draft.trim() === workspace.title || !draft.trim()) {
      setDraft(workspace.title);
      return;
    }
    try {
      onRenamed(await workspaceRename(workspace.slug, draft.trim()));
    } catch (e) {
      setError(String(e));
      setDraft(workspace.title);
    }
  };

  const remove = async (w: WorkspaceMeta) => {
    // A folder full of the user's work, so the confirm says which folder and what goes with it.
    // Nothing here is recoverable and there is no undo to offer.
    const sure = window.confirm(
      `Delete "${w.title}" and everything in it?\n\n${w.path}\n\nThe folder and any work the agents did in it are removed. This cannot be undone.`,
    );
    if (!sure) return;
    try {
      await workspaceDelete(w.slug);
      if (w.slug === workspace.slug) onDeleted();
      else setAll(await workspaceList());
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="identra-ws" ref={root}>
      {editing ? (
        <input
          className="identra-ws__edit"
          autoFocus
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={() => void commitRename()}
          onKeyDown={(e) => {
            if (e.key === "Enter") void commitRename();
            if (e.key === "Escape") {
              setDraft(workspace.title);
              setEditing(false);
            }
          }}
        />
      ) : (
        <button
          className="identra-ws__current"
          title={workspace.path}
          onClick={() => setOpen((v) => !v)}
          onDoubleClick={() => setEditing(true)}
        >
          {workspace.title}
          <span className="identra-ws__caret">▾</span>
        </button>
      )}

      {open && (
        <div className="identra-ws__menu">
          <div className="identra-ws__section">Workspaces</div>
          <ul className="identra-ws__list">
            {all.map((w) => (
              <li key={w.slug} className="identra-ws__row">
                <button
                  className="identra-ws__pick"
                  data-on={w.slug === workspace.slug}
                  onClick={() => {
                    setOpen(false);
                    if (w.slug !== workspace.slug) onOpen(w);
                  }}
                >
                  <span className="identra-ws__name">{w.title}</span>
                  <span className="identra-ws__path">{w.path}</span>
                </button>
                <button
                  className="identra-ws__del"
                  title={`Delete ${w.title}`}
                  onClick={() => void remove(w)}
                >
                  &times;
                </button>
              </li>
            ))}
          </ul>
          <div className="identra-ws__actions">
            <button
              onClick={() => {
                setOpen(false);
                setEditing(true);
              }}
            >
              Rename this one
            </button>
            <button
              onClick={async () => {
                setOpen(false);
                try {
                  onOpen(await workspaceCreate());
                } catch (e) {
                  setError(String(e));
                }
              }}
            >
              New workspace
            </button>
          </div>
          {error && <p className="identra-ws__error">{error}</p>}
        </div>
      )}
    </div>
  );
}

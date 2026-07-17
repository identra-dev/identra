import { useEffect, useState } from "react";
import { workspaceCreate, workspaceList, type WorkspaceMeta } from "./api";

// The first thing you see. A workspace is a folder, so this is really "pick or make a folder", and
// the path under each row says so plainly rather than hiding where the work lands.
export default function WorkspacePicker({
  onOpen,
}: {
  onOpen: (workspace: WorkspaceMeta) => void;
}) {
  const [workspaces, setWorkspaces] = useState<WorkspaceMeta[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    workspaceList().then(setWorkspaces, (e) => {
      setWorkspaces([]);
      setError(String(e));
    });
  }, []);

  const create = async () => {
    setBusy(true);
    setError(null);
    try {
      onOpen(await workspaceCreate());
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  return (
    <div className="identra-picker">
      <div className="identra-picker__panel">
        <h1 className="identra-picker__title">Identra</h1>
        <p className="identra-picker__sub">
          A workspace is a folder. Your canvas lives in it, and so do the agents
          you run.
        </p>

        {workspaces === null ? (
          <p className="identra-picker__empty">
            Looking for your workspaces...
          </p>
        ) : workspaces.length === 0 ? (
          <p className="identra-picker__empty">
            No workspaces yet. Make your first one.
          </p>
        ) : (
          <ul className="identra-picker__list">
            {workspaces.map((w) => (
              <li key={w.slug}>
                <button
                  className="identra-picker__row"
                  onClick={() => onOpen(w)}
                  disabled={busy}
                >
                  <span className="identra-picker__name">{w.title}</span>
                  <span className="identra-picker__path">{w.path}</span>
                </button>
              </li>
            ))}
          </ul>
        )}

        <button
          className="identra-picker__new"
          onClick={create}
          disabled={busy}
        >
          {busy ? "Creating..." : "New workspace"}
        </button>

        {error && <p className="identra-picker__error">{error}</p>}
      </div>
    </div>
  );
}

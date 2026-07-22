import { useEffect, useState, type FormEvent } from "react";
import BoardPreview from "./BoardPreview";
import {
  workspaceClone,
  workspaceCreate,
  workspaceList,
  workspacePickFolder,
  workspaceRecents,
  type WorkspaceMeta,
} from "./api";

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
    // Both kinds in one list: the scratch workspaces Identra made, and the folders you opened. They
    // are the same thing to everything downstream, so they should look the same here too.
    Promise.all([workspaceList(), workspaceRecents()]).then(
      ([made, opened]) => setWorkspaces([...opened, ...made]),
      (e) => {
        setWorkspaces([]);
        setError(String(e));
      },
    );
  }, []);

  const openFolder = async () => {
    setBusy(true);
    setError(null);
    try {
      const picked = await workspacePickFolder();
      // Cancelling the dialog is an answer, not a failure: put the button back and say nothing.
      if (picked) onOpen(picked);
      else setBusy(false);
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

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

  // The clone row: hidden until asked for, because two of three people at this screen have a
  // folder, not a URL, and a permanent input begs to be filled.
  const [cloneOpen, setCloneOpen] = useState(false);
  const [cloneUrl, setCloneUrl] = useState("");
  const [cloning, setCloning] = useState(false);
  const clone = async (e: FormEvent) => {
    e.preventDefault();
    if (!cloneUrl.trim() || cloning) return;
    setCloning(true);
    setBusy(true);
    setError(null);
    try {
      onOpen(await workspaceClone(cloneUrl));
    } catch (err) {
      // Git's own words, verbatim. "Repository not found" from git beats anything this file
      // could paraphrase it into.
      setError(String(err));
      setCloning(false);
      setBusy(false);
    }
  };

  return (
    <div className="identra-picker">
      <div className="identra-picker__panel">
        <h1 className="identra-picker__title">Identra</h1>
        <p className="identra-picker__sub">
          A workspace is a folder. Your canvas lives in it, and so do the agents
          you run, so open the project you actually want them working on.
        </p>

        {workspaces === null ? (
          <p className="identra-picker__empty">
            Looking for your workspaces...
          </p>
        ) : workspaces.length === 0 ? (
          <p className="identra-picker__empty">
            No workspaces yet. Open a folder you are working in, or make an
            empty one to try things out.
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
                  {/* The thumbnail is how you tell two workspaces apart faster than reading
                      paths: the board's own shape, on its own wallpaper. */}
                  <BoardPreview canvas={w.canvas} />
                  <span className="identra-picker__text">
                    <span className="identra-picker__name">{w.title}</span>
                    <span className="identra-picker__path">{w.path}</span>
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}

        <div className="identra-picker__actions">
          {/* The primary action: real work happens in a folder someone already has. */}
          <button
            className="identra-picker__open"
            onClick={() => void openFolder()}
            disabled={busy}
          >
            Open a folder
          </button>
          <button
            className="identra-picker__new"
            onClick={create}
            disabled={busy}
          >
            {busy ? "Working..." : "New empty workspace"}
          </button>
        </div>

        {cloneOpen ? (
          <form className="identra-picker__clone" onSubmit={(e) => void clone(e)}>
            <input
              className="identra-picker__clone-url"
              value={cloneUrl}
              onChange={(e) => setCloneUrl(e.target.value)}
              placeholder="https://github.com/you/your-repo.git"
              aria-label="Repository URL to clone"
              disabled={cloning}
              autoFocus
            />
            <button
              className="identra-picker__clone-go"
              type="submit"
              disabled={cloning || cloneUrl.trim() === ""}
            >
              {cloning ? "Cloning..." : "Clone"}
            </button>
          </form>
        ) : (
          <button
            className="identra-picker__clone-open"
            onClick={() => setCloneOpen(true)}
            disabled={busy}
          >
            Clone a repository instead
          </button>
        )}

        {error && <p className="identra-picker__error">{error}</p>}
      </div>
    </div>
  );
}

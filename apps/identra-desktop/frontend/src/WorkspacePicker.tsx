// The home screen. A navbar carries the three ways in (open a folder, new workspace, clone),
// and the body is a grid of workspace cards, each showing the board itself in miniature on its
// own wallpaper. The grid is the point: you recognize a board by its shape faster than by
// reading paths, so the previews are the biggest thing on the screen.
import { useEffect, useState, type FormEvent } from "react";
import logo from "./assets/identra.png";
import BoardPreview from "./BoardPreview";
import {
  workspaceClone,
  workspaceCreate,
  workspaceList,
  workspacePickFolder,
  workspaceRecents,
  type WorkspaceMeta,
} from "./api";

export default function WorkspacePicker({
  onOpen,
}: {
  onOpen: (workspace: WorkspaceMeta) => void;
}) {
  const [workspaces, setWorkspaces] = useState<WorkspaceMeta[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    // Both kinds in one grid: the scratch workspaces Identra made, and the folders you opened.
    // They are the same thing to everything downstream, so they should look the same here too.
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

  // The clone row: hidden until asked for, because most people at this screen have a folder,
  // not a URL, and a permanent input begs to be filled.
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
    <div className="identra-home">
      <header className="identra-home__nav">
        <img className="identra-logo identra-home__logo" src={logo} alt="" />
        <span className="identra-home__brand">Identra</span>
        <span className="identra-home__tag">
          A workspace is a folder. Your canvas and your agents live in it.
        </span>
        <div className="identra-home__actions">
          <button
            className="identra-home__act"
            data-on={cloneOpen}
            onClick={() => setCloneOpen((v) => !v)}
            disabled={busy && !cloning}
          >
            Clone a repository
          </button>
          <button
            className="identra-home__act"
            onClick={() => void create()}
            disabled={busy}
          >
            {busy && !cloning ? "Working..." : "New workspace"}
          </button>
          <button
            className="identra-home__act identra-home__act--primary"
            onClick={() => void openFolder()}
            disabled={busy}
          >
            Open a folder
          </button>
        </div>
      </header>

      {cloneOpen && (
        <form className="identra-home__clone" onSubmit={(e) => void clone(e)}>
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
      )}

      {error && <p className="identra-home__error">{error}</p>}

      {workspaces === null ? (
        <p className="identra-home__empty">Looking for your workspaces...</p>
      ) : workspaces.length === 0 ? (
        <p className="identra-home__empty">
          No workspaces yet. Open a folder you are working in, or make an empty
          one to try things out. Both live in the bar above.
        </p>
      ) : (
        <ul className="identra-home__grid">
          {workspaces.map((w) => (
            <li key={w.slug}>
              <button
                className="identra-home__card"
                onClick={() => onOpen(w)}
                disabled={busy}
              >
                <BoardPreview
                  canvas={w.canvas}
                  width={280}
                  height={168}
                  className="identra-preview identra-home__preview"
                />
                <span className="identra-home__name">{w.title}</span>
                <span className="identra-home__path">{w.path}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

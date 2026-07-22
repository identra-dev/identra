// The workspace's files, browsable without leaving the board. Same slide-over shape as the Work
// panel. Browsing is lazy, one directory per call; typing a query swaps the listing for search
// hits over names and content. Clicking a file opens it as a viewer node, which is the point:
// this panel exists to look at what the agents made.
import { useEffect, useState } from "react";
import {
  fileReveal,
  filesList,
  filesSearch,
  type FileEntry,
  type FileHit,
} from "./api";

type Props = {
  onOpenFile: (rel: string, name: string) => void;
  onClose: () => void;
};

export default function FilesPanel({ onOpenFile, onClose }: Props) {
  // The directory being browsed, workspace-relative, "" for the root.
  const [dir, setDir] = useState("");
  const [entries, setEntries] = useState<FileEntry[]>([]);
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<FileHit[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    filesList(dir).then(
      (rows) => {
        setEntries(rows);
        setError(null);
      },
      (e) => setError(String(e)),
    );
  }, [dir]);

  // Debounced like the memory search: keystrokes are free, filesystem walks are not.
  useEffect(() => {
    const q = query.trim();
    if (q === "") {
      setHits([]);
      return;
    }
    const timer = window.setTimeout(() => {
      void filesSearch(q)
        .then(setHits)
        .catch((e) => setError(String(e)));
    }, 250);
    return () => window.clearTimeout(timer);
  }, [query]);

  const searching = query.trim() !== "";
  // The crumbs are the way back up. Root first, then every segment of the current path.
  const crumbs = dir === "" ? [] : dir.split("/");

  const reveal = (rel: string) => {
    void fileReveal(rel).catch((e) => setError(String(e)));
  };

  return (
    <aside className="identra-panel">
      <header className="identra-panel__head">
        <div className="identra-panel__tabs">
          <button className="identra-panel__tab" data-on={true}>
            Files
          </button>
        </div>
        <button className="identra-panel__close" onClick={onClose} title="Close">
          &times;
        </button>
      </header>

      <input
        className="identra-panel__search"
        type="search"
        placeholder="Search names and contents"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
      />

      {error !== null && <p className="identra-panel__error">{error}</p>}

      {searching ? (
        hits.length === 0 ? (
          <p className="identra-panel__empty">Nothing matched.</p>
        ) : (
          <ul className="identra-panel__list">
            {hits.map((h) => (
              <li key={`${h.path}:${h.line ?? 0}`} className="identra-fileent">
                <button
                  className="identra-fileent__open"
                  title="Open in a viewer node"
                  onClick={() =>
                    onOpenFile(h.path, h.path.split("/").pop() ?? h.path)
                  }
                >
                  <span className="identra-fileent__name">
                    {h.path}
                    {h.line !== null && (
                      <span className="identra-fileent__line">:{h.line}</span>
                    )}
                  </span>
                  {h.snippet !== null && (
                    <span className="identra-fileent__snippet">{h.snippet}</span>
                  )}
                </button>
                <button
                  className="identra-fileent__reveal"
                  title="Show in your file manager"
                  onClick={() => reveal(h.path)}
                >
                  reveal
                </button>
              </li>
            ))}
          </ul>
        )
      ) : (
        <>
          <nav className="identra-files__crumbs">
            <button
              className="identra-files__crumb"
              onClick={() => setDir("")}
              disabled={dir === ""}
            >
              workspace
            </button>
            {crumbs.map((seg, i) => (
              <button
                key={i}
                className="identra-files__crumb"
                onClick={() => setDir(crumbs.slice(0, i + 1).join("/"))}
                disabled={i === crumbs.length - 1}
              >
                / {seg}
              </button>
            ))}
          </nav>
          {entries.length === 0 ? (
            <p className="identra-panel__empty">This folder is empty.</p>
          ) : (
            <ul className="identra-panel__list">
              {entries.map((e) => (
                <li key={e.path} className="identra-fileent">
                  <button
                    className="identra-fileent__open"
                    title={e.dir ? "Open this folder" : "Open in a viewer node"}
                    onClick={() =>
                      e.dir ? setDir(e.path) : onOpenFile(e.path, e.name)
                    }
                  >
                    <span className="identra-fileent__name">
                      {e.dir ? `${e.name}/` : e.name}
                    </span>
                  </button>
                  {!e.dir && (
                    <button
                      className="identra-fileent__reveal"
                      title="Show in your file manager"
                      onClick={() => reveal(e.path)}
                    >
                      reveal
                    </button>
                  )}
                </li>
              ))}
            </ul>
          )}
        </>
      )}
    </aside>
  );
}

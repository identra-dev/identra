// What the agents have done to your working tree.
//
// The gap this closes is embarrassing when you say it plainly: Identra ran four agents editing your
// repository and gave you no way to see what they touched. The task board showed what they claimed,
// the memory showed what they decided, and finding out what they actually changed meant opening a
// terminal and typing `git status` inside the app you opened to avoid doing that.
//
// Grouped by directory, because a flat list of forty paths is a thing to search rather than read,
// and the shape of a change — six files under `src/memory`, one under `docs` — is most of what a
// person wants from a glance at it.
//
// Read-only, and that is a decision rather than a first cut. See the note on `changes()` in
// identra-core: staging without a commit control is half a gesture, and revert is the only thing in
// this app that destroys work no undo brings back. It would have sat one click from a list you scan
// quickly, describing files an agent wrote while you were not watching.
import { useEffect, useState } from "react";
import { useEscape } from "./useEscape";
import { workspaceChanges, type Changes, type FileChange } from "./api";

// The panel polls, because agents write to the tree from their own processes and there is no event
// to subscribe to without inventing one. Slower than the memory poll: a `git status` walks the
// working tree, which is real work on a large repository, and a diff stat that is four seconds old
// has never mattered to anyone.
const POLL_MS = 4000;

type Props = { onClose: () => void };

// What each state contributes to how the row reads. Deleted is the one that has to look different
// at a glance: everything else is work arriving, and that is work leaving.
const MARK: Record<FileChange["state"], string> = {
  added: "A",
  modified: "M",
  deleted: "D",
  renamed: "R",
  untracked: "?",
};

export default function ChangesPanel({ onClose }: Props) {
  useEscape(onClose);
  const [changes, setChanges] = useState<Changes | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Whether the first read has come back. Without it the panel opens saying a repository with
  // thirty changed files is clean, then corrects itself a beat later — the same lesson the work
  // panel and the file browser both learned, and the empty state is the one moment it is worst to
  // be confidently wrong.
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    let dropped = false;
    const tick = async () => {
      try {
        const next = await workspaceChanges();
        if (dropped) return;
        setChanges(next);
        setError(null);
      } catch (e) {
        if (!dropped) setError(String(e));
      } finally {
        if (!dropped) setLoaded(true);
      }
    };
    void tick();
    const timer = window.setInterval(() => void tick(), POLL_MS);
    return () => {
      dropped = true;
      window.clearInterval(timer);
    };
  }, []);

  // Directory to its files, in the order the engine sorted them, so two reads never reshuffle.
  const groups = new Map<string, FileChange[]>();
  for (const f of changes?.files ?? []) {
    const at = f.path.lastIndexOf("/");
    const dir = at === -1 ? "" : f.path.slice(0, at);
    const list = groups.get(dir);
    if (list === undefined) groups.set(dir, [f]);
    else list.push(f);
  }

  return (
    <div className="identra-panel">
      <div className="identra-panel__head">
        <span className="identra-panel__tab" data-on="true">
          Changes
        </span>
        <button
          className="identra-panel__close"
          onClick={onClose}
          title="Close"
        >
          &times;
        </button>
      </div>

      {changes !== null && (
        // Which branch, and whose. An agent given its own checkout is working somewhere else
        // entirely, and a diff read as your branch when it is a helper's is the kind of wrong that
        // ends with someone committing the wrong thing.
        <div className="identra-changes__branch">
          <span>{changes.branch ?? "detached HEAD"}</span>
          {changes.worktree && (
            <span
              className="identra-changes__worktree"
              title="This checkout is one of Identra's isolated worktrees, not your own branch."
            >
              isolated worktree
            </span>
          )}
        </div>
      )}

      <div className="identra-panel__list">
        {error !== null && (
          <p className="identra-panel__error" role="alert">
            {error}
          </p>
        )}
        {error === null && loaded && groups.size === 0 && (
          <p className="identra-panel__empty">
            Nothing has changed in the working tree.
          </p>
        )}
        {[...groups.entries()].map(([dir, files]) => (
          <div className="identra-changes__group" key={dir}>
            <div className="identra-changes__dir">{dir === "" ? "." : dir}</div>
            {files.map((f) => (
              <div
                className="identra-changes__row"
                key={f.path}
                data-state={f.state}
                // The full path, because the row shows only the last segment and two files called
                // `mod.rs` under different directories are otherwise the same row twice.
                title={f.path}
              >
                <span className="identra-changes__mark">{MARK[f.state]}</span>
                <span className="identra-changes__name">
                  {f.path.slice(f.path.lastIndexOf("/") + 1)}
                </span>
                {f.staged && (
                  <span
                    className="identra-changes__staged"
                    title="Staged. Identra did not stage it — something in your terminal did."
                  >
                    staged
                  </span>
                )}
                {/* A binary file gets no numbers rather than a pair of zeroes: git will not diff
                    it, and "+0 -0" on a 4MB image an agent just wrote reads as nothing happened. */}
                {f.added === null || f.removed === null ? (
                  <span className="identra-changes__binary">binary</span>
                ) : (
                  <span className="identra-changes__stat">
                    <span className="identra-changes__plus">+{f.added}</span>
                    <span className="identra-changes__minus">−{f.removed}</span>
                  </span>
                )}
              </div>
            ))}
          </div>
        ))}
      </div>
    </div>
  );
}

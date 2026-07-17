import { useCallback, useEffect, useState } from "react";
import { boardList, memoryList, type Memory, type Task } from "./api";

// What the agents are coordinating on, for the person watching them.
//
// The board and the memory were agent-only: two terminals scrolled past each other and the user
// could not tell who had taken what, or what the project had already decided. That is the part of
// this product worth watching, so it should not be the part you cannot see.
//
// It polls. The agents write to SQLite from their own processes, so there is no event to subscribe
// to without inventing one, and a two second poll of two small queries is cheaper than the plumbing
// that would avoid it. It only runs while the panel is open.
const POLL_MS = 2000;

type Tab = "tasks" | "memory";

export default function WorkPanel({ onClose }: { onClose: () => void }) {
  const [tab, setTab] = useState<Tab>("tasks");
  const [tasks, setTasks] = useState<Task[]>([]);
  const [memories, setMemories] = useState<Memory[]>([]);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [t, m] = await Promise.all([boardList(), memoryList(50)]);
      setTasks(t);
      setMemories(m);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => void refresh(), POLL_MS);
    return () => window.clearInterval(timer);
  }, [refresh]);

  const state = (t: Task) => {
    if (t.done) return "done";
    if (t.blockedBy.length > 0) return "blocked";
    return t.claimedBy ? "claimed" : "open";
  };

  return (
    <aside className="identra-panel">
      <header className="identra-panel__head">
        <div className="identra-panel__tabs">
          <button
            className="identra-panel__tab"
            data-on={tab === "tasks"}
            onClick={() => setTab("tasks")}
          >
            Work{" "}
            {tasks.length > 0 && (
              <span>{tasks.filter((t) => !t.done).length}</span>
            )}
          </button>
          <button
            className="identra-panel__tab"
            data-on={tab === "memory"}
            onClick={() => setTab("memory")}
          >
            Memory {memories.length > 0 && <span>{memories.length}</span>}
          </button>
        </div>
        <button
          className="identra-panel__close"
          onClick={onClose}
          title="Close"
        >
          &times;
        </button>
      </header>

      {error && <p className="identra-panel__error">{error}</p>}

      {tab === "tasks" ? (
        tasks.length === 0 ? (
          <p className="identra-panel__empty">
            Nothing on the board yet. Agents put work here when they split a
            task, and claim it so two of them never build the same thing.
          </p>
        ) : (
          <ul className="identra-panel__list">
            {tasks.map((t) => (
              <li key={t.id} className="identra-task" data-state={state(t)}>
                <div className="identra-task__top">
                  <span className="identra-task__id">t{t.id}</span>
                  <span className="identra-task__state">{state(t)}</span>
                </div>
                <div className="identra-task__what">{t.description}</div>
                {/* Who has it and what is holding it up are the two things you actually scan for. */}
                {t.claimedBy && !t.done && (
                  <div className="identra-task__meta">
                    taken by {t.claimedBy}
                  </div>
                )}
                {t.blockedBy.length > 0 && (
                  <div className="identra-task__meta">
                    waiting on {t.blockedBy.map((b) => `t${b}`).join(", ")}
                  </div>
                )}
                {t.done && t.note && (
                  <div className="identra-task__meta">{t.note}</div>
                )}
              </li>
            ))}
          </ul>
        )
      ) : memories.length === 0 ? (
        <p className="identra-panel__empty">
          Nothing remembered yet. Agents record decisions and constraints here,
          and every agent you open afterwards starts from them.
        </p>
      ) : (
        <ul className="identra-panel__list">
          {memories.map((m) => (
            <li key={m.id} className="identra-memory">
              {m.content}
            </li>
          ))}
        </ul>
      )}
    </aside>
  );
}

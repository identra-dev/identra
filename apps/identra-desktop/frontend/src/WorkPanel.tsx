import { useCallback, useEffect, useState } from "react";
import {
  boardList,
  memoryForget,
  memoryList,
  memoryRestated,
  memoryModelRetry,
  memorySearch,
  memoryStatus,
  type Memory,
  type ModelStatus,
  type Task,
} from "./api";
import { useEscape } from "./useEscape";
import { ago } from "./ago";

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

export default function WorkPanel({
  onClose,
  initialTab = "tasks",
}: {
  onClose: () => void;
  initialTab?: Tab;
}) {
  const [tab, setTab] = useState<Tab>(initialTab);
  const [tasks, setTasks] = useState<Task[]>([]);
  const [memories, setMemories] = useState<Memory[]>([]);
  // Ids in `memories` that a newer fact already restates. A set rather than a field on the row,
  // because it is a fact about the list and not about the memory: the same row is a restatement or
  // not depending on what else is in view, and the store has no business carrying that.
  const [restated, setRestated] = useState<Set<number>>(new Set());
  const [error, setError] = useState<string | null>(null);
  // Empty means "show everything, newest first" (the polled list). A query switches the memory tab
  // over to ranked search results, so the human can ask the same question an agent would.
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<Memory[]>([]);
  // Null until the engine first answers, so the banner never flickers a wrong state on open. It
  // rides the same poll as the board: the model moves between states on its own (a download
  // finishing, a retry succeeding) with no event to subscribe to.
  const [model, setModel] = useState<ModelStatus | null>(null);
  // Whether the first poll has come back yet.
  //
  // Without this the panel opens telling the truth about an empty array rather than about the
  // project: a workspace with twenty facts painted "Nothing on the board yet" and "Learning this
  // project" with no count, then snapped to the real numbers a poll later. The empty states are
  // the most reassuring copy in the app and showing them to someone whose project is full is the
  // one moment they are actively wrong.
  const [loaded, setLoaded] = useState(false);

  useEscape(onClose);

  // One clock for the whole list, refreshed on the same poll that refreshes the facts. Reading
  // Date.now() per row would make every row disagree slightly and re-render on every tick for no
  // visible gain, since the scale here is coarse enough that a second never changes the words.
  const [nowSeconds, setNowSeconds] = useState(() =>
    Math.floor(Date.now() / 1000),
  );

  const refresh = useCallback(async () => {
    try {
      const [t, m, s, restated] = await Promise.all([
        boardList(),
        memoryList(50),
        // Its own catch, and deliberately not part of the failure above: not knowing what the
        // model is doing is not a reason to stop showing the board and the facts, which is what
        // people have the panel open for.
        memoryStatus().catch(() => null),
        // Same limit as the list, and its own catch for the same reason as the model status: a
        // missing badge is a smaller loss than a panel that will not draw. Failing to an empty set
        // means every row simply renders unmarked, which is what the panel did before.
        memoryRestated(50).catch(() => [] as number[]),
      ]);
      setTasks(t);
      setMemories(m);
      setModel(s);
      setRestated(new Set(restated));
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      // In `finally`, because a first poll that failed has still answered the question this
      // guards: we are no longer in the beat before we knew anything. The error is on screen by
      // then, and leaving a spinner up underneath it would say the opposite.
      setLoaded(true);
    }
  }, []);

  // Remove it here as well as in the engine, so the row goes on the click rather than on the next
  // poll up to two seconds later. A refresh follows to reconcile with what actually happened: if
  // the delete failed, the fact comes straight back, which is the honest outcome rather than a
  // panel that shows a fact gone while every agent still reads it.
  const forget = useCallback(
    async (id: number) => {
      setMemories((cur) => cur.filter((m) => m.id !== id));
      setHits((cur) => cur.filter((m) => m.id !== id));
      try {
        await memoryForget(id);
      } catch (e) {
        setError(String(e));
      }
      void refresh();
    },
    [refresh],
  );

  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => {
      void refresh();
      setNowSeconds(Math.floor(Date.now() / 1000));
    }, POLL_MS);
    return () => window.clearInterval(timer);
  }, [refresh]);

  // Debounced, because with a model attached every search is an embed, and one per keystroke would
  // be wasteful for no gain the eye can see. A blank query clears back to the browse list.
  useEffect(() => {
    const q = query.trim();
    if (q === "") {
      setHits([]);
      return;
    }
    const timer = window.setTimeout(() => {
      void memorySearch(q, 50)
        .then(setHits)
        .catch((e) => setError(String(e)));
    }, 250);
    return () => window.clearTimeout(timer);
  }, [query]);

  const shownMemories = query.trim() === "" ? memories : hits;

  const state = (t: Task) => {
    if (t.done) return "done";
    if (t.blockedBy.length > 0) return "blocked";
    return t.claimedBy ? "claimed" : "open";
  };

  return (
    <aside className="identra-panel">
      <header className="identra-panel__head">
        {/* A real tablist rather than two buttons that look like one. `data-on` drives the paint
            and `aria-selected` says the same thing to a screen reader, which otherwise hears two
            plain buttons and no indication which view is showing. */}
        <div className="identra-panel__tabs" role="tablist" aria-label="Panel">
          <button
            className="identra-panel__tab"
            role="tab"
            aria-selected={tab === "tasks"}
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
            role="tab"
            aria-selected={tab === "memory"}
            data-on={tab === "memory"}
            onClick={() => setTab("memory")}
          >
            Memory {memories.length > 0 && <span>{memories.length}</span>}
          </button>
        </div>
        {/* `title` is a mouse affordance and nothing else: it never reaches a screen reader on a
            button whose only content is a glyph, which is announced as "times" or skipped. */}
        <button
          className="identra-panel__close"
          onClick={onClose}
          title="Close"
          aria-label="Close panel"
        >
          &times;
        </button>
      </header>

      {error && <p className="identra-panel__error">{error}</p>}

      {tab === "tasks" ? (
        !loaded ? (
          // One quiet line rather than skeleton rows: the list is usually short and usually
          // arrives within one frame of the panel opening, so shaped placeholders would flash
          // more than they reassure.
          <p className="identra-panel__empty" role="status">
            Reading the board...
          </p>
        ) : tasks.length === 0 ? (
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
      ) : (
        <>
          {/* The learning state: the moment "any agent you open already knows this project" is
              becoming true, shown as it happens. aria-live polite so the count is announced as
              facts arrive rather than only drawn, without stealing focus from the terminal. */}
          <div className="identra-panel__learning" aria-live="polite">
            {!loaded
              ? "Reading what this project knows..."
              : memories.length === 0
                ? "Learning this project"
                : `Learning this project — ${memories.length} ${
                    memories.length === 1 ? "fact" : "facts"
                  }`}
          </div>
          {/* Why recall might be worse than expected right now, and what to do about it. Only the
              two states worth interrupting for are drawn: "off" is the user's own choice and
              "ready" is the thing working, and narrating either would be noise in a panel people
              keep open. Facts are still recorded and still found by word in every state, so this
              is a degrade notice rather than an error. */}
          {model !== null && model.state === "downloading" && (
            <p className="identra-panel__model" role="status">
              Fetching the recall model (about 130MB). Until it lands, memory
              matches on words. Everything is still being recorded.
            </p>
          )}
          {model !== null && model.state === "failed" && (
            <p
              className="identra-panel__model"
              data-failed="true"
              role="status"
            >
              <span>
                Memory is matching on words: the recall model did not arrive.{" "}
                <span className="identra-panel__model-why">{model.reason}</span>
              </span>
              <button
                className="identra-panel__model-retry"
                onClick={() => void memoryModelRetry().catch(() => {})}
              >
                Retry
              </button>
            </p>
          )}
          <input
            className="identra-panel__search"
            type="search"
            placeholder="Search what the project knows"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
          {/* The browse empty state is gated on `loaded` and the search one is not: a search has
              its own answer the moment it returns, while browsing is the case that was claiming
              an empty project before it had asked. */}
          {shownMemories.length === 0 && (query.trim() !== "" || loaded) ? (
            <p className="identra-panel__empty">
              {query.trim() === ""
                ? "Nothing remembered yet. Agents record decisions and constraints here, and every agent you open afterwards starts from them."
                : "Nothing matched. The closest facts still show for an agent asking over the bus; here, a blank search brings the whole list back."}
            </p>
          ) : shownMemories.length === 0 ? null : (
            <ul className="identra-panel__list">
              {shownMemories.map((m, i) => (
                // The newest fact is the first row when browsing (memory_list is newest-first).
                // Highlight only it, and only when not searching, so a fresh fact catches the eye
                // once and the rest of the list stays calm.
                <li
                  key={m.id}
                  className="identra-memory"
                  data-newest={
                    query.trim() === "" && i === 0 ? true : undefined
                  }
                >
                  <span className="identra-memory__what">{m.content}</span>
                  {/* The fact is the row; this is the footnote that makes it checkable. A list
                      where every line is bare text gives no way to judge one: an agent's guess
                      from three sessions ago reads exactly like something decided this morning.
                      Who and when are the two axes anyone scans a memory list on. */}
                  <span className="identra-memory__meta">
                    <span>{ago(m.created_at, nowSeconds)}</span>
                    {/* Why the same decision appears more than once. Three agents write down that
                        the queue moved to postgres in three wordings, every one a real row, and the
                        panel showed all three with nothing to say they were one thing. Marking the
                        older wordings is what turns a list that looks like it is losing track of
                        itself into a list that is visibly keeping the newest version of each thing.
                        Only in the browse list: search ranks by relevance, so "newer" has no
                        meaning there and the mark would be pointing at nothing. */}
                    {query.trim() === "" && restated.has(m.id) && (
                      <>
                        <span aria-hidden="true">·</span>
                        <span
                          className="identra-memory__restated"
                          title="A newer fact above says this too. Agents are told the newest wording, so this one is not sent again."
                        >
                          restated more recently
                        </span>
                      </>
                    )}
                    {m.run_id !== "" && (
                      <>
                        <span aria-hidden="true">·</span>
                        <span>{m.run_id}</span>
                      </>
                    )}
                    <button
                      className="identra-memory__forget"
                      onClick={() => void forget(m.id)}
                      title="Forget this fact"
                      aria-label={`Forget: ${m.content}`}
                    >
                      Forget
                    </button>
                  </span>
                </li>
              ))}
            </ul>
          )}
        </>
      )}
    </aside>
  );
}

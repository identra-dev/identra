//! The shared task board.
//!
//! Two agents splitting work by talking is fine until they both pick the same piece, or both wait
//! for the other, or one forgets what it agreed to. A board fixes all three: the work is written
//! down where every agent in the workspace can see it, claiming is atomic so a task can only be
//! taken once, and a task can declare what has to finish before it is workable.
//!
//! It is SQLite, in the workspace, for one reason: claiming has to be atomic. Several agents call
//! `claim_task` at the same moment from different threads, and a read then write over a JSON file
//! would hand the same task to both. A conditional UPDATE cannot: exactly one caller sees a row
//! change. That is the whole argument for a database here, and it is enough of one.
//!
//! A claim is not a lock forever. If the agent holding a task is gone, the task returns to the pool
//! rather than stranding the work, because the alternative is a board that silently stops.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS tasks (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    description  TEXT    NOT NULL,
    claimed_by   TEXT,
    done_at      INTEGER,
    note         TEXT,
    created_at   INTEGER NOT NULL
);
-- A row here means `task_id` cannot start until `after_id` is done.
CREATE TABLE IF NOT EXISTS task_deps (
    task_id  INTEGER NOT NULL,
    after_id INTEGER NOT NULL,
    PRIMARY KEY (task_id, after_id)
);
";

/// What an agent needs to decide whether to take a task.
#[derive(Debug, PartialEq)]
pub struct Task {
    pub id: i64,
    pub description: String,
    pub claimed_by: Option<String>,
    pub done: bool,
    pub note: Option<String>,
    /// Ids that must be done first. Non-empty and unfinished means this is not workable yet.
    pub blocked_by: Vec<i64>,
}

pub struct Board {
    conn: Connection,
}

impl Board {
    pub fn open(project_dir: &Path) -> Result<Board, String> {
        let conn = crate::open_bus_db(project_dir)?;
        conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
        Ok(Board { conn })
    }

    /// Post work. `after` names tasks that must finish first; an id that does not exist is rejected
    /// rather than stored, because a dependency on nothing would silently never unblock.
    pub fn add(&self, description: &str, after: &[i64], now: i64) -> Result<i64, String> {
        let description = description.trim();
        if description.is_empty() {
            return Err("a task needs a description".into());
        }
        for dep in after {
            let exists: Option<i64> = self
                .conn
                .query_row("SELECT id FROM tasks WHERE id = ?1", [dep], |r| r.get(0))
                .optional()
                .map_err(|e| e.to_string())?;
            if exists.is_none() {
                return Err(format!("cannot depend on t{dep}: no such task"));
            }
        }
        self.conn
            .execute(
                "INSERT INTO tasks (description, created_at) VALUES (?1, ?2)",
                params![description, now],
            )
            .map_err(|e| e.to_string())?;
        let id = self.conn.last_insert_rowid();
        for dep in after {
            self.conn
                .execute(
                    "INSERT OR IGNORE INTO task_deps (task_id, after_id) VALUES (?1, ?2)",
                    params![id, dep],
                )
                .map_err(|e| e.to_string())?;
        }
        Ok(id)
    }

    pub fn list(&self) -> Result<Vec<Task>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, description, claimed_by, done_at, note FROM tasks ORDER BY id")
            .map_err(|e| e.to_string())?;
        let rows: Vec<Task> = stmt
            .query_map([], |r| {
                Ok(Task {
                    id: r.get(0)?,
                    description: r.get(1)?,
                    claimed_by: r.get(2)?,
                    done: r.get::<_, Option<i64>>(3)?.is_some(),
                    note: r.get(4)?,
                    blocked_by: Vec::new(),
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        drop(stmt);

        let mut out = Vec::with_capacity(rows.len());
        for mut task in rows {
            task.blocked_by = self.unfinished_deps(task.id)?;
            out.push(task);
        }
        Ok(out)
    }

    /// Dependencies of `id` that are not done yet. Empty means the task is workable.
    fn unfinished_deps(&self, id: i64) -> Result<Vec<i64>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT d.after_id FROM task_deps d
                 JOIN tasks t ON t.id = d.after_id
                 WHERE d.task_id = ?1 AND t.done_at IS NULL
                 ORDER BY d.after_id",
            )
            .map_err(|e| e.to_string())?;
        let ids = stmt
            .query_map([id], |r| r.get(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<i64>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(ids)
    }

    /// Take a task. With no id, take the oldest workable one.
    ///
    /// `live` is the set of nodes still running. A task held by a node that is gone is free again:
    /// an agent that crashed mid-task should not strand that work forever. The UPDATE carries the
    /// whole claimable condition, so two agents racing on the same task cannot both win it, and
    /// exactly one sees a row change.
    pub fn claim(&self, node: &str, id: Option<i64>, live: &[String]) -> Result<Task, String> {
        let wanted = match id {
            Some(id) => id,
            None => self
                .list()?
                .into_iter()
                .find(|t| !t.done && t.blocked_by.is_empty() && self.claimable_by(t, node, live))
                .map(|t| t.id)
                .ok_or_else(|| {
                    "no workable task: the board is empty, blocked, or all taken".to_string()
                })?,
        };

        let task = self.get(wanted)?;
        if task.done {
            return Err(format!("t{wanted} is already done"));
        }
        if !task.blocked_by.is_empty() {
            let blockers = join_ids(&task.blocked_by);
            return Err(format!("t{wanted} is blocked until {blockers} is done"));
        }
        if !self.claimable_by(&task, node, live) {
            let holder = task.claimed_by.unwrap_or_default();
            return Err(format!("t{wanted} is already claimed by {holder}"));
        }

        // The condition lives in the statement, not in the check above, because between that check
        // and this line another agent may have claimed it. Only the caller that changes a row wins.
        let holder = task.claimed_by.clone().unwrap_or_default();
        let changed = self
            .conn
            .execute(
                "UPDATE tasks SET claimed_by = ?1
                 WHERE id = ?2 AND done_at IS NULL
                   AND (claimed_by IS NULL OR claimed_by = ?3)",
                params![node, wanted, holder],
            )
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err(format!("t{wanted} was taken by someone else just now"));
        }
        self.get(wanted)
    }

    /// Free to take: unclaimed, already mine, or held by a node that is no longer running.
    fn claimable_by(&self, task: &Task, node: &str, live: &[String]) -> bool {
        match &task.claimed_by {
            None => true,
            Some(holder) => holder == node || !live.iter().any(|l| l == holder),
        }
    }

    pub fn get(&self, id: i64) -> Result<Task, String> {
        let mut task = self
            .conn
            .query_row(
                "SELECT id, description, claimed_by, done_at, note FROM tasks WHERE id = ?1",
                [id],
                |r| {
                    Ok(Task {
                        id: r.get(0)?,
                        description: r.get(1)?,
                        claimed_by: r.get(2)?,
                        done: r.get::<_, Option<i64>>(3)?.is_some(),
                        note: r.get(4)?,
                        blocked_by: Vec::new(),
                    })
                },
            )
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("no task t{id}"))?;
        task.blocked_by = self.unfinished_deps(id)?;
        Ok(task)
    }

    /// Finish a task and report what that unblocked, so the agent knows there is more to pick up
    /// rather than assuming the board is done with it.
    pub fn complete(&self, id: i64, note: Option<&str>, now: i64) -> Result<Vec<Task>, String> {
        let task = self.get(id)?;
        if task.done {
            return Err(format!("t{id} is already done"));
        }
        self.conn
            .execute(
                "UPDATE tasks SET done_at = ?1, note = ?2 WHERE id = ?3",
                params![now, note, id],
            )
            .map_err(|e| e.to_string())?;

        // Anything that depended on this and now has nothing left blocking it.
        let mut stmt = self
            .conn
            .prepare("SELECT task_id FROM task_deps WHERE after_id = ?1 ORDER BY task_id")
            .map_err(|e| e.to_string())?;
        let dependents = stmt
            .query_map([id], |r| r.get::<_, i64>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<i64>, _>>()
            .map_err(|e| e.to_string())?;
        drop(stmt);

        let mut unblocked = Vec::new();
        for dep in dependents {
            let t = self.get(dep)?;
            if !t.done && t.blocked_by.is_empty() {
                unblocked.push(t);
            }
        }
        Ok(unblocked)
    }
}

fn join_ids(ids: &[i64]) -> String {
    ids.iter()
        .map(|i| format!("t{i}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// One board line, written for an agent deciding what to do next rather than for a UI.
pub fn render(task: &Task) -> String {
    let state = if task.done {
        match &task.note {
            Some(n) if !n.is_empty() => format!("done: {n}"),
            _ => "done".to_string(),
        }
    } else if !task.blocked_by.is_empty() {
        format!("blocked by {}", join_ids(&task.blocked_by))
    } else {
        match &task.claimed_by {
            Some(who) => format!("claimed by {who}"),
            None => "open, claimable".to_string(),
        }
    };
    format!("- t{} [{}] {}", task.id, state, task.description)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board(name: &str) -> (Board, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("identra-tasks-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        (Board::open(&dir).unwrap(), dir)
    }

    #[test]
    fn a_task_can_only_be_claimed_once() {
        let (b, dir) = board("claim");
        let t1 = b.add("write the route", &[], 100).unwrap();
        let live = vec!["a".to_string(), "b".to_string()];

        assert_eq!(
            b.claim("a", Some(t1), &live).unwrap().claimed_by.as_deref(),
            Some("a")
        );
        // b racing for the same task loses rather than both agents building the route.
        assert!(b
            .claim("b", Some(t1), &live)
            .unwrap_err()
            .contains("already claimed by a"));
        // Claiming my own task again is not an error, it is the same state.
        assert!(b.claim("a", Some(t1), &live).is_ok());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_dead_agents_claim_does_not_strand_the_work() {
        let (b, dir) = board("dead");
        let t1 = b.add("write the route", &[], 100).unwrap();
        b.claim("a", Some(t1), &["a".into()]).unwrap();

        // a is gone from the live set, so its claim is not a lock on the board forever.
        let taken = b.claim("b", Some(t1), &["b".into()]).unwrap();
        assert_eq!(taken.claimed_by.as_deref(), Some("b"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn dependencies_gate_claiming_and_completion_reports_what_opened_up() {
        let (b, dir) = board("deps");
        let route = b.add("write the route", &[], 100).unwrap();
        let test = b.add("test the route", &[route], 100).unwrap();
        let live = vec!["a".to_string()];

        // The test cannot start before the route exists, and the refusal says why.
        let err = b.claim("a", Some(test), &live).unwrap_err();
        assert!(err.contains("blocked until t1"), "got: {err}");
        // Claiming with no id skips the blocked one and hands over the workable one.
        assert_eq!(b.claim("a", None, &live).unwrap().id, route);

        let unblocked = b
            .complete(route, Some("GET /health returns ok"), 200)
            .unwrap();
        assert_eq!(unblocked.len(), 1);
        assert_eq!(unblocked[0].id, test, "finishing the route opens the test");

        assert!(b
            .complete(route, None, 300)
            .unwrap_err()
            .contains("already done"));
        assert!(b.claim("a", None, &live).unwrap().id == test);

        // A dependency on a task that does not exist never unblocks, so it is refused at write time.
        assert!(b
            .add("impossible", &[999], 100)
            .unwrap_err()
            .contains("no such task"));
        assert!(b.add("   ", &[], 100).is_err());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_board_line_says_what_an_agent_needs_to_decide() {
        let (b, dir) = board("render");
        let t1 = b.add("write the route", &[], 100).unwrap();
        let t2 = b.add("test the route", &[t1], 100).unwrap();

        assert_eq!(
            render(&b.get(t1).unwrap()),
            "- t1 [open, claimable] write the route"
        );
        assert_eq!(
            render(&b.get(t2).unwrap()),
            "- t2 [blocked by t1] test the route"
        );

        b.claim("a", Some(t1), &["a".into()]).unwrap();
        assert_eq!(
            render(&b.get(t1).unwrap()),
            "- t1 [claimed by a] write the route"
        );

        b.complete(t1, Some("returns ok"), 200).unwrap();
        assert_eq!(
            render(&b.get(t1).unwrap()),
            "- t1 [done: returns ok] write the route"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }
}

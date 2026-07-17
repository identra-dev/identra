//! identra-memory: the memory layer as one small library over a single SQLite file.
//!
//! The public surface, all scoped by `(user_id, agent_id, run_id)`: [`Store::add`],
//! [`Store::search`], [`Store::get`], [`Store::update`], [`Store::delete`], and
//! [`Store::history`]. `add` runs text through an [`Extractor`] (verbatim by default), drops any
//! fact already held for this `(user, agent)`, optionally embeds it, writes the row, and records
//! the change in an append only transition log.
//!
//! The heavy pieces are seams, so the crate stays offline and easy to test. With no [`Embedder`]
//! set, `search` matches on substring. With the default verbatim [`Extractor`], the whole blob is
//! one fact. A local embedding model plugs into [`Embedder`]; the user's agent model plugs into
//! [`Extractor`]. That is the "no model configured means store verbatim, never block" behavior the
//! plan asks for.

use std::fmt;
use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// Who a memory belongs to. All three fields are required on a write. [`Filter`] makes them
/// optional on a read.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Scope {
    pub user_id: String,
    pub agent_id: String,
    pub run_id: String,
}

/// A read side scope. Any field left `None` is a wildcard, so `Filter::default()` matches
/// everything.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Filter {
    pub user_id: Option<String>,
    pub agent_id: Option<String>,
    pub run_id: Option<String>,
}

/// One stored fact. `created_at` marks when it was first learned; `update` bumps `updated_at`.
/// Both are unix seconds.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Memory {
    pub id: i64,
    pub user_id: String,
    pub agent_id: String,
    pub run_id: String,
    pub content: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// What happened to a memory. Stored as its lowercase name in the history log.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Event {
    Added,
    Updated,
    Deleted,
}

impl Event {
    fn as_str(self) -> &'static str {
        match self {
            Event::Added => "added",
            Event::Updated => "updated",
            Event::Deleted => "deleted",
        }
    }

    fn parse(s: &str) -> Event {
        match s {
            "updated" => Event::Updated,
            "deleted" => Event::Deleted,
            _ => Event::Added,
        }
    }
}

/// One line of the transition log: what changed, and the text before and after. `before` is
/// `None` on an add, `after` is `None` on a delete. This is what gives audit and undo.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Change {
    pub id: i64,
    pub memory_id: i64,
    pub event: Event,
    pub before: Option<String>,
    pub after: Option<String>,
    pub created_at: i64,
}

/// Turn text into a vector for semantic search. A local model (for example fastembed) plugs in
/// here. With no embedder set, [`Store::search`] falls back to substring matching.
pub trait Embedder: Send + Sync {
    fn embed(&self, text: &str) -> Vec<f32>;
}

/// Pull the durable facts out of a raw blob of session text. The default ([`Verbatim`]) keeps the
/// whole trimmed blob as one fact; a model backed impl returns several distilled facts.
pub trait Extractor: Send + Sync {
    fn extract(&self, text: &str) -> Vec<String>;
}

/// The no model extractor: the trimmed text is one fact, empty text is nothing.
pub struct Verbatim;

impl Extractor for Verbatim {
    fn extract(&self, text: &str) -> Vec<String> {
        let t = text.trim();
        if t.is_empty() {
            vec![]
        } else {
            vec![t.to_string()]
        }
    }
}

#[derive(Debug)]
pub enum Error {
    Db(rusqlite::Error),
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Db(e)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Db(e) => write!(f, "db error: {e}"),
        }
    }
}

impl std::error::Error for Error {}

/// The store. It owns one SQLite connection. rusqlite's methods take `&self`, so no interior
/// `Mutex` is needed here; a caller that shares the store across threads wraps the whole `Store`.
pub struct Store {
    conn: Connection,
    embedder: Option<Box<dyn Embedder>>,
    extractor: Box<dyn Extractor>,
}

impl Store {
    /// Open (creating if absent) a store at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Store, Error> {
        Self::from_conn(Connection::open(path)?)
    }

    /// An in memory store, for tests and for the no project open case.
    pub fn open_in_memory() -> Result<Store, Error> {
        Self::from_conn(Connection::open_in_memory()?)
    }

    fn from_conn(conn: Connection) -> Result<Store, Error> {
        conn.execute_batch(SCHEMA)?;
        Ok(Store {
            conn,
            embedder: None,
            extractor: Box::new(Verbatim),
        })
    }

    /// Attach an embedder, so `search` ranks by cosine similarity instead of substring.
    pub fn with_embedder(mut self, embedder: Box<dyn Embedder>) -> Self {
        self.embedder = Some(embedder);
        self
    }

    /// Replace the default verbatim extractor with a model backed one.
    pub fn with_extractor(mut self, extractor: Box<dyn Extractor>) -> Self {
        self.extractor = extractor;
        self
    }

    /// Extract facts from `text`, store the ones not already held for this `(user, agent)`, and
    /// return only the newly stored memories. Re learning a known fact is a silent no op.
    pub fn add(&self, scope: &Scope, text: &str) -> Result<Vec<Memory>, Error> {
        let now = unix_now();
        let mut stored = Vec::new();
        for fact in self.extractor.extract(text) {
            let embedding = self.embedder.as_ref().map(|e| pack(&e.embed(&fact)));
            // Dedup is UNIQUE(user, agent, content) plus INSERT OR IGNORE. run_id is left out of
            // the key on purpose, so the same fact learned in a new run does not duplicate, which
            // is the point of memory. The btree index is the content hash.
            let changed = self.conn.execute(
                "INSERT OR IGNORE INTO memories
                     (user_id, agent_id, run_id, content, embedding, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                params![
                    scope.user_id,
                    scope.agent_id,
                    scope.run_id,
                    fact,
                    embedding,
                    now
                ],
            )?;
            if changed == 0 {
                continue; // already held: no duplicate row, no history noise
            }
            let id = self.conn.last_insert_rowid();
            self.record(id, Event::Added, None, Some(&fact), now)?;
            stored.push(Memory {
                id,
                user_id: scope.user_id.clone(),
                agent_id: scope.agent_id.clone(),
                run_id: scope.run_id.clone(),
                content: fact,
                created_at: now,
                updated_at: now,
            });
        }
        Ok(stored)
    }

    /// The most relevant memories in `filter`'s scope. With an embedder: cosine over the embedded
    /// rows, best first. Without one: rows whose content contains `query` (case insensitive), most
    /// recently touched first. Capped at `limit`.
    pub fn search(&self, filter: &Filter, query: &str, limit: usize) -> Result<Vec<Memory>, Error> {
        let rows = self.scoped_rows(filter)?;
        let ranked = match &self.embedder {
            Some(embedder) => {
                let q = embedder.embed(query);
                // Brute force cosine over the scoped set. A personal store is hundreds to low
                // thousands of rows, so a real vector index (sqlite-vec) is premature; add one when
                // a scope routinely tops ten thousand rows. Rows added with no embedder have no
                // vector and are skipped here, so keep one embedder config per store.
                let mut scored: Vec<(f32, Row)> = rows
                    .into_iter()
                    .filter_map(|r| r.vector().map(|v| (cosine(&q, &v), r)))
                    .collect();
                scored.sort_by(|a, b| b.0.total_cmp(&a.0));
                scored.into_iter().map(|(_, r)| r).collect::<Vec<_>>()
            }
            None => {
                let needle = query.to_lowercase();
                let mut hits: Vec<Row> = rows
                    .into_iter()
                    .filter(|r| r.content.to_lowercase().contains(&needle))
                    .collect();
                hits.sort_by_key(|r| std::cmp::Reverse(r.updated_at));
                hits
            }
        };
        Ok(ranked
            .into_iter()
            .take(limit)
            .map(Row::into_memory)
            .collect())
    }

    /// Fetch one memory by id.
    pub fn get(&self, id: i64) -> Result<Option<Memory>, Error> {
        let row = self
            .conn
            .query_row(
                SELECT_COLS.replace("{where}", "id = ?1").as_str(),
                [id],
                Row::from_sql,
            )
            .optional()?;
        Ok(row.map(Row::into_memory))
    }

    /// Revise a memory's text in place, re embedding it, and log the transition. Returns the
    /// updated memory, or `None` if the id is unknown. Identical new text is a no op.
    pub fn update(&self, id: i64, new_content: &str) -> Result<Option<Memory>, Error> {
        let Some(existing) = self.get(id)? else {
            return Ok(None);
        };
        let after = new_content.trim();
        if after == existing.content {
            return Ok(Some(existing)); // nothing changed, no history noise
        }
        let now = unix_now();
        let embedding = self.embedder.as_ref().map(|e| pack(&e.embed(after)));
        self.conn.execute(
            "UPDATE memories SET content = ?1, embedding = ?2, updated_at = ?3 WHERE id = ?4",
            params![after, embedding, now, id],
        )?;
        self.record(
            id,
            Event::Updated,
            Some(&existing.content),
            Some(after),
            now,
        )?;
        self.get(id)
    }

    /// Delete a memory and log the deletion. Returns whether a row was actually removed. The
    /// deleted text stays in `history`, so it is recoverable.
    pub fn delete(&self, id: i64) -> Result<bool, Error> {
        let Some(m) = self.get(id)? else {
            return Ok(false);
        };
        self.conn
            .execute("DELETE FROM memories WHERE id = ?1", [id])?;
        self.record(id, Event::Deleted, Some(&m.content), None, unix_now())?;
        Ok(true)
    }

    /// The transition log for one memory, oldest first.
    pub fn history(&self, memory_id: i64) -> Result<Vec<Change>, Error> {
        let mut stmt = self.conn.prepare(
            "SELECT id, memory_id, event, before, after, created_at
             FROM history WHERE memory_id = ?1 ORDER BY id ASC",
        )?;
        let rows = stmt
            .query_map([memory_id], |r| {
                Ok(Change {
                    id: r.get(0)?,
                    memory_id: r.get(1)?,
                    event: Event::parse(&r.get::<_, String>(2)?),
                    before: r.get(3)?,
                    after: r.get(4)?,
                    created_at: r.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn record(
        &self,
        memory_id: i64,
        event: Event,
        before: Option<&str>,
        after: Option<&str>,
        at: i64,
    ) -> Result<(), Error> {
        self.conn.execute(
            "INSERT INTO history (memory_id, event, before, after, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![memory_id, event.as_str(), before, after, at],
        )?;
        Ok(())
    }

    /// Every row in `filter`'s scope. A `None` filter field is a wildcard, written as
    /// `(?n IS NULL OR col = ?n)` so one prepared statement covers all filter shapes.
    fn scoped_rows(&self, filter: &Filter) -> Result<Vec<Row>, Error> {
        let sql = SELECT_COLS.replace(
            "{where}",
            "(?1 IS NULL OR user_id = ?1)
               AND (?2 IS NULL OR agent_id = ?2)
               AND (?3 IS NULL OR run_id = ?3)",
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(
                params![filter.user_id, filter.agent_id, filter.run_id],
                Row::from_sql,
            )?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

/// The on disk shape, read straight out of a query. Kept internal; callers see [`Memory`].
struct Row {
    id: i64,
    user_id: String,
    agent_id: String,
    run_id: String,
    content: String,
    embedding: Option<Vec<u8>>,
    created_at: i64,
    updated_at: i64,
}

impl Row {
    fn from_sql(r: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
        Ok(Row {
            id: r.get(0)?,
            user_id: r.get(1)?,
            agent_id: r.get(2)?,
            run_id: r.get(3)?,
            content: r.get(4)?,
            embedding: r.get(5)?,
            created_at: r.get(6)?,
            updated_at: r.get(7)?,
        })
    }

    fn vector(&self) -> Option<Vec<f32>> {
        self.embedding.as_deref().map(unpack)
    }

    fn into_memory(self) -> Memory {
        Memory {
            id: self.id,
            user_id: self.user_id,
            agent_id: self.agent_id,
            run_id: self.run_id,
            content: self.content,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

/// Shared column list for reads. `{where}` is filled per query so `get` and `scoped_rows` stay in
/// sync on column order, which `Row::from_sql` depends on.
const SELECT_COLS: &str =
    "SELECT id, user_id, agent_id, run_id, content, embedding, created_at, updated_at
     FROM memories WHERE {where}";

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS memories (
    id         INTEGER PRIMARY KEY,
    user_id    TEXT NOT NULL,
    agent_id   TEXT NOT NULL,
    run_id     TEXT NOT NULL,
    content    TEXT NOT NULL,
    embedding  BLOB,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS memories_dedup ON memories(user_id, agent_id, content);

CREATE TABLE IF NOT EXISTS history (
    id         INTEGER PRIMARY KEY,
    memory_id  INTEGER NOT NULL,
    event      TEXT NOT NULL,
    before     TEXT,
    after      TEXT,
    created_at INTEGER NOT NULL
);
";

/// f32 vector to little endian bytes for the BLOB column, and back.
fn pack(v: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(v.len() * 4);
    for f in v {
        bytes.extend_from_slice(&f.to_le_bytes());
    }
    bytes
}

fn unpack(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// Cosine similarity. Zero for a zero length vector, which avoids a NaN, and it compares over the
/// shared prefix so a dimension mismatch degrades instead of panicking.
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let (mut dot, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
    for i in 0..a.len().min(b.len()) {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(run: &str) -> Scope {
        Scope {
            user_id: "u".into(),
            agent_id: "codex".into(),
            run_id: run.into(),
        }
    }

    #[test]
    fn add_dedupes_across_runs_then_get_and_delete() {
        let store = Store::open_in_memory().unwrap();

        let added = store.add(&scope("r1"), "we chose axum over actix").unwrap();
        assert_eq!(added.len(), 1);
        let id = added[0].id;

        // Same fact, a later run: no duplicate, nothing returned as new.
        let again = store
            .add(&scope("r2"), "  we chose axum over actix  ")
            .unwrap();
        assert!(again.is_empty());

        // get round trips; provenance stays with the first learn.
        let got = store.get(id).unwrap().unwrap();
        assert_eq!(got.content, "we chose axum over actix");
        assert_eq!(got.run_id, "r1");

        // delete removes it and is idempotent.
        assert!(store.delete(id).unwrap());
        assert!(!store.delete(id).unwrap());
        assert!(store.get(id).unwrap().is_none());

        // Empty text stores nothing.
        assert!(store.add(&scope("r1"), "   ").unwrap().is_empty());
    }

    #[test]
    fn update_revises_in_place() {
        let store = Store::open_in_memory().unwrap();
        let id = store.add(&scope("r1"), "cache ttl is 12h").unwrap()[0].id;

        let updated = store.update(id, "cache ttl is 24h").unwrap().unwrap();
        assert_eq!(updated.content, "cache ttl is 24h");
        assert!(updated.updated_at >= updated.created_at);

        // Identical text is a no op: still one history add plus one update, not two updates.
        let same = store.update(id, "cache ttl is 24h").unwrap().unwrap();
        assert_eq!(same.content, "cache ttl is 24h");

        // Unknown id yields None, not an error.
        assert!(store.update(9999, "nope").unwrap().is_none());
    }

    #[test]
    fn history_records_the_transitions() {
        let store = Store::open_in_memory().unwrap();
        let id = store.add(&scope("r1"), "old fact").unwrap()[0].id;
        store.update(id, "new fact").unwrap();
        store.delete(id).unwrap();

        let log = store.history(id).unwrap();
        assert_eq!(log.len(), 3);
        assert_eq!(log[0].event, Event::Added);
        assert_eq!(log[0].before, None);
        assert_eq!(log[0].after.as_deref(), Some("old fact"));
        assert_eq!(log[1].event, Event::Updated);
        assert_eq!(log[1].before.as_deref(), Some("old fact"));
        assert_eq!(log[1].after.as_deref(), Some("new fact"));
        assert_eq!(log[2].event, Event::Deleted);
        assert_eq!(log[2].before.as_deref(), Some("new fact"));
        assert_eq!(log[2].after, None);
    }

    #[test]
    fn text_search_without_a_model() {
        let store = Store::open_in_memory().unwrap();
        store.add(&scope("r1"), "the cache uses a 24h TTL").unwrap();
        store.add(&scope("r1"), "auth is JWT with JWKS").unwrap();

        let hits = store.search(&Filter::default(), "CACHE", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].content.contains("cache"));
    }

    // A deterministic stand in for a real model: a vector of keyword counts.
    struct KeywordEmbedder;
    impl Embedder for KeywordEmbedder {
        fn embed(&self, text: &str) -> Vec<f32> {
            let t = text.to_lowercase();
            ["rust", "python", "cache", "auth"]
                .iter()
                .map(|k| t.matches(k).count() as f32)
                .collect()
        }
    }

    #[test]
    fn semantic_search_ranks_by_cosine() {
        let store = Store::open_in_memory()
            .unwrap()
            .with_embedder(Box::new(KeywordEmbedder));
        store.add(&scope("r1"), "the rust build is slow").unwrap();
        store
            .add(&scope("r1"), "python typing is optional")
            .unwrap();

        let hits = store
            .search(&Filter::default(), "rust toolchain", 2)
            .unwrap();
        assert_eq!(hits[0].content, "the rust build is slow");
    }

    #[test]
    fn scope_filter_narrows_results() {
        let store = Store::open_in_memory().unwrap();
        store
            .add(
                &Scope {
                    user_id: "u".into(),
                    agent_id: "codex".into(),
                    run_id: "r1".into(),
                },
                "codex fact",
            )
            .unwrap();
        store
            .add(
                &Scope {
                    user_id: "u".into(),
                    agent_id: "claude".into(),
                    run_id: "r2".into(),
                },
                "claude fact",
            )
            .unwrap();

        let only_other = Filter {
            agent_id: Some("claude".into()),
            ..Default::default()
        };
        let hits = store.search(&only_other, "fact", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].content, "claude fact");
    }
}

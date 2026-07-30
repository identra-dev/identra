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
use std::sync::Arc;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

// Public because the shell has to name one thing in it: the env var it hands the bundled model's
// path through. Everything else here is an implementation detail of `LocalEmbedder`, which is
// re-exported below and is what callers actually construct.
#[cfg(feature = "fastembed")]
pub mod local_embedder;
#[cfg(feature = "fastembed")]
pub use local_embedder::LocalEmbedder;

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
    /// The embedding model could not be loaded. Recall still works, on words rather than meaning,
    /// so this is a downgrade to report and not a reason to fail a caller who only wanted to write
    /// a memory down.
    Model(String),
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
            Error::Model(e) => write!(f, "embedding model unavailable: {e}"),
        }
    }
}

impl std::error::Error for Error {}

/// The store. It owns one SQLite connection. rusqlite's methods take `&self`, so no interior
/// `Mutex` is needed here; a caller that shares the store across threads wraps the whole `Store`.
pub struct Store {
    conn: Connection,
    /// Shared rather than owned, because loading a model costs a second and a store is opened per
    /// call. One model, many stores.
    embedder: Option<Arc<dyn Embedder>>,
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
        // WAL plus a busy timeout, set at the one gate every caller passes through. This file is
        // opened per call from several places at once: an agent writing a fact over the bus, the
        // work panel polling every two seconds, and a fresh agent reading the recent facts at
        // connect. What the default rollback journal costs is concurrency rather than correctness,
        // since rusqlite already defaults the busy timeout to five seconds and a second writer
        // waits rather than erroring. WAL is what lets the poll read while an agent writes instead
        // of the two taking turns, and it matters most when the canvas is busiest. The timeout is
        // pinned here rather than inherited so the guarantee belongs to this function instead of to
        // a dependency's default, which can change under us without the build saying anything.
        // On an in-memory store WAL is a no-op, which is fine: nothing shares that handle.
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000;")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Store {
            conn,
            embedder: None,
            extractor: Box::new(Verbatim),
        })
    }

    /// Attach an embedder, so `search` ranks by cosine similarity instead of substring, and give
    /// any row that predates it a vector.
    ///
    /// The backfill is the whole reason this returns a Result. A cosine search can only rank rows
    /// it has a vector for, so a memory written while no embedder was set is not merely ranked
    /// badly, it is unreachable, and it stays unreachable forever while looking perfectly healthy
    /// in the work panel. That happens on the ordinary path: someone runs offline before the model
    /// is fetched, writes down three decisions, and later cannot find them. Embedding on the way in
    /// keeps the rule simple, every row in a store with an embedder has a vector, and the rule is
    /// what makes the search honest.
    pub fn with_embedder(mut self, embedder: Arc<dyn Embedder>) -> Result<Self, Error> {
        self.embedder = Some(embedder);
        self.backfill_vectors()?;
        Ok(self)
    }

    /// Embed every row that has no vector. Cheap and idempotent after the first pass, since it only
    /// looks at rows where the column is null.
    fn backfill_vectors(&self) -> Result<(), Error> {
        let Some(embedder) = self.embedder.as_ref() else {
            return Ok(());
        };
        let mut stmt = self
            .conn
            .prepare("SELECT id, content FROM memories WHERE embedding IS NULL")?;
        let pending: Vec<(i64, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        for (id, content) in pending {
            self.conn.execute(
                "UPDATE memories SET embedding = ?1 WHERE id = ?2",
                params![pack(&embedder.embed(&content)), id],
            )?;
        }
        Ok(())
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

    /// The most recently learned memories in scope, newest first, capped at `limit`.
    ///
    /// Browsing, not searching. [`search`](Store::search) answers "what do we know about X" and
    /// needs a question; this answers "what has been learned here", which is what someone reading
    /// over their agents' shoulder actually wants, and there is no query to ask.
    pub fn recent(&self, filter: &Filter, limit: usize) -> Result<Vec<Memory>, Error> {
        let mut rows = self.scoped_rows(filter)?;
        rows.sort_by_key(|r| std::cmp::Reverse(r.created_at));
        Ok(rows.into_iter().take(limit).map(Row::into_memory).collect())
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

/// How alike two facts have to read before the second one is not worth its tokens. Jaccard over
/// word trigrams, so 0.6 means most of one's phrasing appears in the other.
///
/// Tuned to catch restatement, not topic. "we dropped redis for postgres listen/notify" against
/// "the job queue moved off redis to postgres listen/notify" scores well over this; two different
/// decisions that both mention postgres score far under it. Erring low here costs a few tokens;
/// erring high loses a fact, so the threshold sits where it does deliberately.
const NEAR_DUPLICATE: f32 = 0.6;

/// Word trigrams, lowercased. Short texts fall back to their words, so a fact of one or two words
/// still has something to compare.
fn shingles(text: &str) -> std::collections::HashSet<String> {
    let words: Vec<String> = text
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect();
    if words.len() < 3 {
        return words.into_iter().collect();
    }
    words.windows(3).map(|w| w.join(" ")).collect()
}

/// Every run of digits in a fact, each as its own token. `us-east-1` gives `1`, `block13` gives
/// `13`, and `v0.1.2` gives `0`, `1` and `2`.
///
/// Runs of digits rather than parsed numbers on purpose. The question this answers is only "do these
/// two sentences disagree about a figure", and treating `08` and `8` as a disagreement is the safe
/// direction to be wrong in: it keeps a fact that might be a restatement, instead of dropping one
/// that is not.
fn digits(text: &str) -> std::collections::HashSet<String> {
    let mut found = std::collections::HashSet::new();
    let mut run = String::new();
    for c in text.chars() {
        if c.is_ascii_digit() {
            run.push(c);
        } else if !run.is_empty() {
            found.insert(std::mem::take(&mut run));
        }
    }
    if !run.is_empty() {
        found.insert(run);
    }
    found
}

/// Drop facts that restate one already kept, preserving order.
///
/// A memory store accumulates the same decision in several wordings: three agents each record that
/// the queue moved to postgres, in three sentences, and every one of them is a real distinct row
/// because the content hash that dedupes exact repeats cannot see that they mean the same thing.
/// Injecting all three into every agent's context at connect spends tokens to say one thing three
/// times, and it does it once per agent per session, which is where this actually costs.
///
/// Lexical rather than semantic on purpose. Connect deliberately opens the store without an
/// embedder so the handshake never waits on the model, so there are no vectors here to cluster and
/// this has to work without them. Shingle overlap catches restatement, which is the shape that
/// accumulates, and it costs a hash set per fact against a list already capped at twenty.
///
/// The first wording of a thing wins, and the caller hands these in newest first, so what survives
/// is the most recent way the project put it.
///
/// Figures are checked before phrasing, and a disagreement about one settles it. "deploy the worker
/// to region us-east-1" and "...us-east-2" share every trigram but the last, which is a Jaccard of
/// exactly 0.6 and therefore a collapse under the threshold above — two real, different decisions,
/// and the second silently gone from every agent's connect payload with nothing to show it happened.
/// A sentence whose whole content is a number cannot be deduped on the words around the number.
///
/// So: if two facts name different figures, they are different facts whatever their wording. If they
/// name the same figures, or neither names any, phrasing decides as before. One naming a figure and
/// the other not counts as a disagreement, which errs toward keeping a fact, the direction this
/// whole function is tuned to err in.
pub fn drop_near_duplicates(facts: Vec<String>) -> Vec<String> {
    let survives = near_duplicate_mask(&facts);
    facts
        .into_iter()
        .zip(survives)
        .filter(|(_, keep)| *keep)
        .map(|(fact, _)| fact)
        .collect()
}

/// Which of these facts survive the collapse, position by position. `true` is kept, `false` is
/// already said by an earlier one.
///
/// The decision lives here and [`drop_near_duplicates`] is a filter over it, so there is one
/// implementation rather than two that agree until someone edits one of them.
///
/// This exists because the collapse runs on the connect payload and nowhere else, so the work panel
/// was listing every restatement while the agents received one. That reads as memory repeating
/// itself, and worse, it means the panel could not say which facts an agent will actually be told.
/// Filtering the panel to match would have been the smaller change and the wrong one: a hidden fact
/// is a fact nobody can delete, and a store the user cannot correct is the thing the panel exists to
/// prevent. So the panel keeps every row and marks the ones that do not travel.
pub fn near_duplicate_mask(facts: &[String]) -> Vec<bool> {
    let mut kept: Vec<(
        std::collections::HashSet<String>,
        std::collections::HashSet<String>,
    )> = Vec::new();
    let mut survives = Vec::with_capacity(facts.len());
    for fact in facts {
        let shape = shingles(fact);
        // Nothing to compare and nothing to say. Treated as not surviving, which is what
        // `drop_near_duplicates` has always done with it.
        if shape.is_empty() {
            survives.push(false);
            continue;
        }
        let figures = digits(fact);
        let redundant = kept.iter().any(|(seen, seen_figures)| {
            if *seen_figures != figures {
                return false;
            }
            let overlap = shape.intersection(seen).count() as f32;
            let union = shape.union(seen).count() as f32;
            union > 0.0 && overlap / union >= NEAR_DUPLICATE
        });
        survives.push(!redundant);
        if !redundant {
            kept.push((shape, figures));
        }
    }
    survives
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod dedup_tests {
    use super::drop_near_duplicates;

    /// This runs on every agent's connect payload, so what it drops and what it keeps is a token
    /// bill on one side and a fact the project loses on the other.
    #[test]
    fn restatement_goes_and_distinct_decisions_stay() {
        let facts = vec![
            "The job queue moved off redis to postgres listen/notify".to_string(),
            // The same decision, recorded by another agent in its own words. This is the case that
            // actually accumulates, and the one worth paying a hash set per fact to catch.
            "the job queue moved off redis to postgres listen notify.".to_string(),
            // Also about postgres, and not the same fact at all. Topic overlap must not read as
            // restatement or the store loses real decisions.
            "Postgres runs in docker for local development".to_string(),
            "The API issues JWT bearer tokens rather than server side sessions".to_string(),
        ];
        let kept = drop_near_duplicates(facts);
        assert_eq!(
            kept,
            vec![
                "The job queue moved off redis to postgres listen/notify".to_string(),
                "Postgres runs in docker for local development".to_string(),
                "The API issues JWT bearer tokens rather than server side sessions".to_string(),
            ],
            "the restatement goes, everything that says something new stays"
        );
    }

    /// Two facts that differ only in a number are two facts, and the number is the whole content.
    ///
    /// This is the failure mode the threshold comment warns about from the other side: erring high
    /// loses a fact. Trigram Jaccard cannot see that `us-east-1` and `us-east-2` are the point of
    /// the sentence rather than a detail of it, so a pair of real, different decisions scores as
    /// restatement and the second one is dropped out of every agent's connect payload — silently,
    /// and with no way for anyone to notice it happened.
    #[test]
    fn facts_differing_only_by_a_number_both_survive() {
        let kept = drop_near_duplicates(vec![
            "deploy the worker to region us-east-1".to_string(),
            "deploy the worker to region us-east-2".to_string(),
        ]);
        assert_eq!(
            kept,
            vec![
                "deploy the worker to region us-east-1".to_string(),
                "deploy the worker to region us-east-2".to_string(),
            ],
            "a differing number is a differing fact, not a restatement"
        );

        // The same shape without the numbers still has to collapse, or the guard has simply turned
        // near-duplicate detection off for anything with a digit in it.
        let kept = drop_near_duplicates(vec![
            "the job queue moved off redis to postgres listen/notify".to_string(),
            "the job queue moved off redis to postgres listen notify.".to_string(),
        ]);
        assert_eq!(kept.len(), 1, "restatement without numbers still collapses");

        // A figure both facts agree on is not a reason to keep a restatement. The guard only ever
        // blocks on disagreement, so a fact carrying a number still collapses against its own
        // rewording, and the number riding along changes nothing.
        let kept = drop_near_duplicates(vec![
            "the api listens on port 8080 in development".to_string(),
            "The API listens on port 8080 in development.".to_string(),
        ]);
        assert_eq!(
            kept.len(),
            1,
            "an agreed figure leaves restatement collapsing as before"
        );
    }

    #[test]
    fn order_is_kept_and_short_facts_survive() {
        // Newest first is what the caller hands in, so the first wording of a thing is the most
        // recent way the project put it, and that is the one to keep.
        let kept = drop_near_duplicates(vec![
            "newest".to_string(),
            "middle".to_string(),
            "oldest".to_string(),
        ]);
        assert_eq!(kept, vec!["newest", "middle", "oldest"]);

        // Under three words there are no trigrams. Falling back to the words themselves is what
        // stops a short fact hashing to nothing and being silently dropped.
        let kept = drop_near_duplicates(vec![
            "ship it".to_string(),
            "ship it".to_string(),
            "rust only".to_string(),
        ]);
        assert_eq!(kept, vec!["ship it", "rust only"]);

        // Nothing in, nothing out, and a fact with no words in it is not a fact.
        assert!(drop_near_duplicates(Vec::new()).is_empty());
        assert!(drop_near_duplicates(vec!["   ".to_string()]).is_empty());
    }
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
            .with_embedder(Arc::new(KeywordEmbedder))
            .unwrap();
        store.add(&scope("r1"), "the rust build is slow").unwrap();
        store
            .add(&scope("r1"), "python typing is optional")
            .unwrap();

        let hits = store
            .search(&Filter::default(), "rust toolchain", 2)
            .unwrap();
        assert_eq!(hits[0].content, "the rust build is slow");
    }

    /// A memory written before there was an embedder must not become unreachable once there is
    /// one. Cosine can only rank a row it has a vector for, so without the backfill this fact is
    /// silently invisible to every future search while still sitting in the work panel looking
    /// fine. That is the shape of the bug I care most about here: not a wrong answer, a confident
    /// empty one.
    #[test]
    fn a_fact_learned_before_the_model_is_still_findable_after_it() {
        let store = Store::open_in_memory().unwrap();
        store.add(&scope("r1"), "the rust build is slow").unwrap();

        let store = store.with_embedder(Arc::new(KeywordEmbedder)).unwrap();
        let hits = store
            .search(&Filter::default(), "rust toolchain", 2)
            .unwrap();
        assert_eq!(
            hits.first().map(|m| m.content.as_str()),
            Some("the rust build is slow"),
            "a row written before the embedder was attached is still searchable"
        );
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

    /// A file store opens in WAL with a busy timeout, so the panel poll and an agent's write run
    /// alongside each other instead of taking turns. Pinning the pragmas here catches a regression
    /// that would otherwise only show as a slow canvas under load, which is where nobody looks.
    #[test]
    fn a_file_store_opens_in_wal_with_a_busy_timeout() {
        let dir = std::env::temp_dir().join(format!("identra-wal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(dir.join("memory.db")).unwrap();

        let mode: String = store
            .conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal");
        let timeout: i64 = store
            .conn
            .query_row("PRAGMA busy_timeout", [], |r| r.get(0))
            .unwrap();
        assert!(timeout >= 5000, "the busy timeout is set, got {timeout}");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The real thing the pragmas buy: a reader on one connection keeps working while a writer on
    /// another hammers the same file. Two hundred rounds of it, because the failure this guards
    /// against is intermittent by nature, and one round of each would pass either way.
    #[test]
    fn a_reader_is_not_locked_out_while_a_writer_works() {
        use std::sync::{Arc, Barrier};
        use std::thread;

        let dir = std::env::temp_dir().join(format!("identra-wal2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("memory.db");
        // A fact to read before the writer starts, so the reader always has a row.
        Store::open(&path)
            .unwrap()
            .add(&scope("r"), "seed")
            .unwrap();

        let gate = Arc::new(Barrier::new(2));
        let writer = {
            let path = path.clone();
            let gate = gate.clone();
            thread::spawn(move || {
                let store = Store::open(&path).unwrap();
                gate.wait();
                for i in 0..200 {
                    store.add(&scope("r"), &format!("w {i}")).unwrap();
                }
            })
        };

        let reader = Store::open(&path).unwrap();
        gate.wait();
        for _ in 0..200 {
            // Panics if this ever comes back "database is locked", which is the whole point.
            reader.recent(&Filter::default(), 10).unwrap();
        }
        writer.join().unwrap();

        std::fs::remove_dir_all(&dir).unwrap();
    }
}

# identra-memory

The memory layer as a lean Rust library: `add` / `search` / `get` / `delete`, scoped by
`(user_id, agent_id, run_id)`. Fact extraction is a single model call that falls back to storing
the text verbatim when no model is configured; entries are deduped by content hash, kept in an
append-only SQLite history, embedded locally, and searched over a vector index. No graph, no
reranker.

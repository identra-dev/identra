# identra-memory

The memory layer as a lean Rust library: `add` / `search` / `get` / `update` / `delete` /
`history`, scoped by `(user_id, agent_id, run_id)`. Fact extraction is a single model call that
falls back to storing the text verbatim when no model is configured. Entries are deduped by
content hash and embedded locally through a pluggable seam. Search ranks by vector similarity when
an embedder is set, and by substring otherwise. Every change writes an append-only transition row
(event, before, after) to a `history` table, which gives audit and undo. No graph, no reranker.

Two seams keep the heavy parts optional and offline: `Embedder` (text to vector, e.g. fastembed)
and `Extractor` (text to facts, e.g. the user's agent model). Both are absent by default, so the
crate builds and runs with no model and no network.

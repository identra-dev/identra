# Memory

Every coding agent forgets everything when you close it. Memory is Identra's answer: it keeps the
parts of a session worth keeping and hands them to the next agent you open, so nobody starts from
zero and you stop re-explaining the same three constraints every morning.

This page is what it keeps, how recall works, where it lives, and how to turn it off.

## What it keeps

After a session, one extraction pass pulls out the durable facts and drops the rest. Durable means
the things that would still be true next week and that a new teammate would need told:

- decisions, and what they were decided against
- directions that were tried and rejected
- conventions this project follows that are not written down anywhere else

Not the transcript. Not every message. A log of everything said is not memory, it is a log, and it
is no more useful to the next agent than the terminal scrollback it came from.

Facts are deduped by content hash, so the same decision restated five times across five sessions
is stored once.

## Recall works on meaning, not on words

Ask "how do we handle auth" and you get back the decision that was written down as "the API issues
JWT bearer tokens" — two phrasings that share no word at all.

That runs on a small embedding model on your machine. A release build ships the model inside it, so
recall works by meaning the first time you open Identra, offline, with nothing to download and
nothing to wait for.

A build from source has no bundled model and fetches one into your OS cache under `identra/models`,
once.

### What recall will not do

A search hands back the closest facts it has and says that is what they are. It does not claim they
are the answer.

The ranking is good and its scores still cannot tell a real answer from a question about something
this project never touched, so a confidence threshold would only ever be a guess wearing a number.
Judging whether a recalled fact is relevant is the agent's job, and the agent is better at it than
a cutoff would be.

## Reading and writing it

Agents reach memory through the bus, the same MCP connection they use to talk to each other. They
can search it and add to it.

You can read all of it, too. The work panel lists every fact the project holds, and each one can be
deleted. That is deliberate: memory only agents can read is memory you cannot check or correct, and
a wrong fact that quietly persists is worse than no memory at all.

On a fresh agent in a project Identra already knows, what it remembers is shown before the agent's
first prompt.

## Turning it off

```bash
IDENTRA_EMBEDDINGS=off
```

Set that and recall falls back to matching words instead of meaning. Worse results, no model, and
nothing but your own files involved. There is also a toggle in Settings.

The engine reads this once per process, so a change made in Settings lands at the next launch, and
the panel says so rather than pretending it took effect.

## Where it lives

| What | Where |
|------|-------|
| The facts | `.identra/memory.db` in the project, plus `-wal` and `-shm` while it is open |
| The model | inside a release build; else your OS cache under `identra/models` |

One SQLite file per workspace with a vector index. It never leaves your machine, and your memories
are never part of the model.

The `-wal` and `-shm` files are SQLite's own. They exist because Identra runs in WAL mode so the
panel can read the board while an agent writes to it, and they come and go on their own. Adding
`.identra/` to your `.gitignore` covers all of it.

## When the model has a problem

Memory degrades quietly and never blocks your work. It will not stop a session to tell you a model
failed.

The one exception is a failure you can act on: the memory panel names it and offers a retry, rather
than mentioning it once and moving on. If no model is configured at all, facts are stored as raw
text instead of guessing at an embedding.

## See also

- [Agents](./agents.md) — why Claude Code is the one that shows you memory on connect
- [Troubleshooting](./troubleshooting.md) — a model that will not load

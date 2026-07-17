# Recipe: two agents build a health route together

This is the smallest recipe that shows two wired agent nodes sharing context over the bus. One
agent writes a `GET /health` route, the other writes the test for it, and they hand off through
the three bus tools instead of a human copy-pasting between terminals.

## The setup

1. Drop two agent nodes on the canvas. The lead is a Codex node. The partner can be any agent
   you have installed.
2. Draw an edge between them. The edge is the authorization: without it the bus refuses every
   peer call, so wire first.
3. Launch both nodes. Codex registers the bus tools at startup, so the wire has to exist before
   launch, not after.
4. Paste the lead prompt into the first node and the partner prompt into the second.

## Why the file split matters

Both agents work in the same project directory and there is no worktree isolation yet, so two
agents editing one file will overwrite each other. The recipe avoids that by giving each agent
its own path: the lead owns `src/health.rs` (or your project's route file), the partner owns
`tests/health_test.rs`. They never touch the same file, so their edits never collide.

## The three bus tools

Each agent has these once it launches wired to a peer:

- `list_peers()` returns the node ids joined to you by an edge.
- `get_peer_context(nodeId)` returns the recent transcript of that peer, so you can read what
  they just did.
- `send_to_node(nodeId, text)` pushes a line into that peer's terminal, prefixed with your name.

## Lead prompt (the route author, Codex)

```
You are wired to one partner node on the canvas. Call list_peers() to get its id.

Your job: add a GET /health endpoint that returns 200 with the JSON body {"status":"ok"}.
Edit only the route file (src/health.rs or the project's equivalent). Do not touch any test
file, your partner owns those.

When the route compiles, call send_to_node(<partnerId>, "health route is on GET /health,
returns {\"status\":\"ok\"}, please write and run the test") so your partner can test it.
Then wait for their reply and fix the route if their test finds a problem.
```

## Partner prompt (the test author)

```
You are wired to one partner node on the canvas. Call list_peers() to get its id.

Your job: write a test for the partner's GET /health endpoint and run it. Edit only the test
file (tests/health_test.rs or the project's equivalent). Do not touch the route file, your
partner owns it.

First call get_peer_context(<partnerId>) to read what route they built. Write the test, run it,
and call send_to_node(<partnerId>, "<pass or the failure output>") so they know the result. If
it fails, keep the message specific enough that they can fix the route.
```

## What the audience sees

Two real terminals, a lit edge between them, and messages crossing as each agent works: the
lead announces the route, the partner reads the context, writes the test, runs it, and reports
back. No human retyping anything between the two.

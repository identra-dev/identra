# Identra tasks. Run `just` with no args to see this list.
set shell := ["bash", "-uc"]

# show all tasks
default:
    @just --list

# run the app with hot reload
dev:
    cd apps/identra-desktop && cargo tauri dev

# build a release bundle (AppImage + .deb on linux)
build:
    cd apps/identra-desktop && cargo tauri build

# run the whole test suite
test:
    # No model in the tests. A workspace build turns the embedding feature on for every crate,
    # because the app asks for it, and a suite that downloads 130MB is a suite that fails on a
    # train. Word matching is the path this exercises, and it is a real path: it is what someone
    # offline gets. Meaning based ranking is checked by hand with the recall-check recipe.
    IDENTRA_EMBEDDINGS=off cargo test --workspace
    cd apps/identra-desktop/frontend && bun test

# See recall work against the real model. Fetches it on the first run, then works offline.
recall-check:
    cargo run -p identra-memory --features fastembed --example recall_check

# format rust and web
fmt:
    cargo fmt --all
    cd apps/identra-desktop/frontend && bun run fmt

# lint everything, warnings fail the build
lint:
    cargo clippy --workspace -- -D warnings
    cd apps/identra-desktop/frontend && bun run lint

# fail if any AI/tool attribution trace is in the source
trace-gate:
    bash scripts/trace-gate.sh

# the gate i run before every commit: format, lint, test, trace-gate
check: fmt lint test trace-gate

# check this machine has what it needs to build and run
doctor:
    @command -v cargo >/dev/null && echo "rust    ok" || echo "rust    MISSING"
    @command -v cargo-tauri >/dev/null && echo "tauri   ok" || echo "tauri   MISSING (cargo install tauri-cli)"
    @command -v bun >/dev/null && echo "bun     ok" || echo "bun     MISSING"
    @command -v codex >/dev/null && echo "codex   ok" || echo "codex   MISSING (needed to run an agent node)"

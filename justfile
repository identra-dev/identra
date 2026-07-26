# Identra tasks. Run `just` with no args to see this list.
set shell := ["bash", "-uc"]

# show all tasks
default:
    @just --list

# run the app with hot reload
#
# Clears its own port first. Vite is pinned to 1420 with strictPort, because the Tauri window loads
# that exact address and a dev server that quietly moves to 1421 gives you a blank window instead of
# an app. The cost of pinning is that anything still holding 1420 makes every later run fail the
# same way, and a `just dev` that dies partway leaves cargo-tauri, vite and the built app behind — so
# one interrupted run poisons every run after it, with an error that names a port and not a cause.
# That is a genuinely confusing half hour the first time, and it is nobody's fault twice.
dev:
    @lsof -ti tcp:1420 2>/dev/null | xargs -r kill 2>/dev/null || true
    cd apps/identra-desktop && cargo tauri dev

# fetch the embedding model into the bundle's resources (once; a release build ships it)
fetch-model:
    ./apps/identra-desktop/fetch-model.sh

# build a release bundle (AppImage + .deb on linux), with the model in it
build: fetch-model
    cd apps/identra-desktop && cargo tauri build

# run the whole test suite
test:
    # No model in the tests. A workspace build turns the embedding feature on for every crate,
    # because the app asks for it, and a suite that downloads 130MB is a suite that fails on a
    # train. Word matching is the path this exercises, and it is a real path: it is what someone
    # offline gets. Meaning based ranking is checked by hand with the recall-check recipe.
    IDENTRA_EMBEDDINGS=off cargo test --workspace
    # The production build too, not just the tests. Type errors that only vite's build surfaces are
    # otherwise found by CI on the tag rather than here.
    cd apps/identra-desktop/frontend && bun run build && bun test

# See recall work against the real model. Fetches it on the first run, then works offline.
recall-check:
    cargo run -p identra-memory --features fastembed --example recall_check

# See agent discovery under the stripped PATH a Dock or Finder launch hands the app, which is the
# case that shipped broken once: CLIs that resolve in a terminal were invisible to a bundled build.
# The binary is built first and then run under the bare env, because cargo itself needs the PATH
# this is taking away. Every installed agent should still print an absolute cmd.
detect-check:
    cargo build -p identra-core --example detect_probe
    env -i HOME="$HOME" PATH=/usr/bin:/bin ./target/debug/examples/detect_probe

# format rust and web
fmt:
    cargo fmt --all
    cd apps/identra-desktop/frontend && bun run fmt

# lint everything, warnings fail the build
lint:
    # --all-targets, because without it clippy skips test code and CI does not. A lint that only
    # fails on the tag is a lint that fails at the worst possible moment, so this matches
    # .github/workflows/build.yml exactly.
    cargo clippy --workspace --all-targets -- -D warnings
    cd apps/identra-desktop/frontend && bun run lint

# the gate i run before every commit: format, lint, test
check: fmt lint test

# check this machine has what it needs to build and run
doctor:
    @command -v cargo >/dev/null && echo "rust    ok" || echo "rust    MISSING"
    @command -v cargo-tauri >/dev/null && echo "tauri   ok" || echo "tauri   MISSING (cargo install tauri-cli)"
    @command -v bun >/dev/null && echo "bun     ok" || echo "bun     MISSING"
    @command -v claude >/dev/null && echo "claude  ok" || echo "claude  MISSING (claude.com/claude-code)"
    @command -v codex >/dev/null && echo "codex   ok" || echo "codex   missing (optional)"
    @command -v gemini >/dev/null && echo "gemini  ok" || echo "gemini  missing (optional)"
    @command -v opencode >/dev/null && echo "opencode ok" || echo "opencode missing (optional)"
    @echo "an agent node needs at least one of the four; claude is the one that gets the project's memory on connect"

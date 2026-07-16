#!/usr/bin/env bash
# Fail if any AI/tool attribution trace is in the source.
#
# Catches the two mechanical trace sources: skill authoring markers and Claude
# commit trailers. Deliberately narrow — it does NOT flag bare "claude"/"anthropic",
# so naming a supported agent later won't trip it. Widen the pattern if a new trace
# source appears.
#
# Scoped to the working tree and honors .gitignore (skips reference/, target/,
# node_modules/, dist/). --no-index lets it run before the first commit.
set -uo pipefail

pattern='ponytail|co-authored-by:[[:space:]]*claude|generated with \[?claude'

# ':!scripts/trace-gate.sh' keeps the pattern below from matching this file itself.
if git grep --no-index --exclude-standard -nIiE "$pattern" -- . ':!scripts/trace-gate.sh'; then
    echo >&2
    echo "trace-gate: attribution traces found above — strip them before committing." >&2
    exit 1
fi
echo "trace-gate ok"

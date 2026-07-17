#!/usr/bin/env bash
# Fail if the source carries a tell it should not.
#
# Two separate checks, because they fail for different reasons.
#
# 1. Attribution traces: tool authoring markers and generated-commit trailers. Deliberately narrow:
#    it does NOT flag bare "claude", so naming a supported agent in the registry will not trip it.
#    Widen the pattern only if a new mechanical trace source appears.
#
# 2. Em and en dashes. This is in the house rules for a reason and it is the one style rule worth
#    mechanising: it is invisible in review, it is the loudest generated-text tell there is, and a
#    comma, a colon or two sentences always says the same thing. Prose is the point of catching it,
#    but I scan everything, since a dash in a doc comment ships just as surely as one in a README.
#
# Scoped to the working tree and honors .gitignore (skips reference/, target/, node_modules/,
# dist/). --no-index lets it run before the first commit.
set -uo pipefail

# ':!scripts/trace-gate.sh' keeps the patterns below from matching this file itself.
readonly SELF=':!scripts/trace-gate.sh'
fail=0

attribution='ponytail|co-authored-by:[[:space:]]*claude|generated with \[?claude'
if git grep --no-index --exclude-standard -nIiE "$attribution" -- . "$SELF"; then
    echo >&2
    echo "trace-gate: attribution traces found above, strip them before committing." >&2
    fail=1
fi

# -P for a unicode escape; the em dash is U+2014 and the en dash U+2013.
if git grep --no-index --exclude-standard -nIP '[\x{2013}\x{2014}]' -- . "$SELF"; then
    echo >&2
    echo "trace-gate: em or en dash found above. Use a comma, a colon, or two sentences." >&2
    fail=1
fi

[ "$fail" -eq 0 ] || exit 1
echo "trace-gate ok"

#!/usr/bin/env bash
# Put the embedding model where the bundler will pick it up.
#
# Identra ships the model rather than fetching it on first use, so recall works by meaning the
# moment someone opens the app: offline, with nothing to wait for. That means the files have to be
# on disk before `cargo tauri build` runs, which is what this does.
#
# It is not part of `just dev`. A source checkout falls back to fetching the model into the OS cache
# on first use, which is the path contributors already had, and staging 130MB before you can run the
# app once is a bad first day.
#
# The names and the source are fastembed's own, so a bundled directory holds exactly what a
# downloaded one would: same repo, same files, same layout.
set -euo pipefail

REPO="Xenova/bge-small-en-v1.5"
DEST="$(cd "$(dirname "$0")" && pwd)/src-tauri/resources/model"
BASE="https://huggingface.co/${REPO}/resolve/main"

# The network is the model file; the rest are small json the tokenizer is built from.
FILES=(
  "onnx/model.onnx:model.onnx"
  "tokenizer.json:tokenizer.json"
  "config.json:config.json"
  "special_tokens_map.json:special_tokens_map.json"
  "tokenizer_config.json:tokenizer_config.json"
)

mkdir -p "$DEST"
for entry in "${FILES[@]}"; do
  remote="${entry%%:*}"
  local="${entry##*:}"
  target="$DEST/$local"
  # Already there is already done. A rebuild should not re-download 130MB, and CI caches this
  # directory between runs for the same reason.
  if [ -s "$target" ]; then
    echo "have    $local"
    continue
  fi
  echo "fetch   $local"
  curl -fL --retry 3 --retry-delay 2 -o "$target.part" "$BASE/$remote"
  # Rename only after a complete download, so an interrupted fetch cannot leave a truncated file
  # that the "already there" check above would then skip forever.
  mv "$target.part" "$target"
done

# A truncated onnx file loads as a broken session rather than as an error anywhere useful, so the
# one check worth having is that the big file is actually big.
size=$(wc -c <"$DEST/model.onnx")
if [ "$size" -lt 50000000 ]; then
  echo "model.onnx is only $size bytes, which is not the whole model" >&2
  exit 1
fi
echo "model ready in $DEST ($((size / 1024 / 1024))MB)"

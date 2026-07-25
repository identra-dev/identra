# The embedding model goes here

Identra ships the model that makes recall work by meaning, so a new install has it on first
launch: offline, nothing to wait for, nothing fetched. `fetch-model.sh` in the app directory puts
the files here, and the bundler picks them up from the glob in `tauri.conf.json`.

```
../../fetch-model.sh      # from apps/identra-desktop/src-tauri/resources/model
just fetch-model          # or, from the repo root
```

The weights are not in this repository and never should be: they are 130MB and they are not ours.
This file is here so the bundler's glob has something to match, which is what lets `just dev` and
a plain `cargo build` work without staging the model first. When the files are missing the app
falls back to fetching the model into the OS cache on first use, exactly as it did before it was
bundled.

The model is [BAAI/bge-small-en-v1.5](https://huggingface.co/BAAI/bge-small-en-v1.5), MIT
licensed, fetched from the ONNX conversion at `Xenova/bge-small-en-v1.5` that fastembed itself
uses. See `NOTICE` at the repository root.

Files a complete directory holds, all of them named the way fastembed names them:

| File | What it is |
|------|-----------|
| `model.onnx` | The network. This is the 130MB |
| `tokenizer.json` | The tokenizer |
| `config.json`, `special_tokens_map.json`, `tokenizer_config.json` | What the tokenizer is built from |

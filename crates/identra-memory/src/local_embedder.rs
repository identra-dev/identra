//! A local embedding model, so recall matches on meaning instead of on shared words.
//!
//! Without this, `search` compares strings. A fact stored as "the API issues JWT bearer tokens"
//! and a question asked as "how do we handle auth" have no word in common, so the search misses,
//! and a miss on the one feature the product is built around is not a small thing. Meaning is the
//! only thing that closes that gap: no amount of stemming or scoring gets from "auth" to "JWT".
//!
//! The model runs on the machine. Identra's promise is that a project's memory stays local, and
//! shipping recall off to an embedding API for every search would quietly break it. The cost of
//! keeping that promise is a model on disk, which is the trade I want.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use fastembed::{
    EmbeddingModel, InitOptions, InitOptionsUserDefined, Pooling, QuantizationMode, TextEmbedding,
    TokenizerFiles, UserDefinedEmbeddingModel,
};

use crate::{Embedder, Error};

/// Where a build that ships the model tells us to find it. Set by the desktop shell from its own
/// bundled resources before anything asks for an embedder.
///
/// An env var rather than a parameter because this crate knows nothing about Tauri, app bundles, or
/// where an installer put things, and should not have to. What it knows is: here is a directory, or
/// there is not one.
pub const MODEL_DIR_ENV: &str = "IDENTRA_MODEL_DIR";

/// The files a bundled model is made of: the network, and the four the tokenizer is built from.
/// These are the names fastembed fetches from the hub, kept identical so a bundled directory and a
/// downloaded one hold the same thing under the same names.
const ONNX: &str = "model.onnx";
const TOKENIZER_FILES: [&str; 4] = [
    "tokenizer.json",
    "config.json",
    "special_tokens_map.json",
    "tokenizer_config.json",
];

/// The model I default to. Small (about 130MB on disk), a few milliseconds per fact on a CPU, and
/// 384 dimensions, which is plenty for a store this size. A bigger model ranks a little better and
/// costs a much longer first run, which is the wrong trade for an app someone just installed.
///
/// I compared it against AllMiniLML6V2 on the questions in `examples/recall_check.rs`, worded the
/// way an agent would ask rather than the way the fact was written. This model put the right fact
/// first on all four. MiniLM got three, and it ranked an unrelated question about kubernetes above
/// a correct match on auth, which is the failure that matters: not a weaker score, the wrong fact
/// on top.
///
/// Its scores are not a relevance signal, and that is worth knowing before anyone adds a cutoff
/// here. Measured on those same questions, a right answer scored 0.53 to 0.68 and a question about
/// something this project has never heard of still scored up to 0.60. The ranges overlap, so there
/// is no floor that keeps the junk out without also dropping real answers. Prefixing the query with
/// the instruction BGE documents for retrieval made the overlap worse, not better. The ordering is
/// what this model is good at, so ordering is all I take from it. See `search` for what that means.
const MODEL: EmbeddingModel = EmbeddingModel::BGESmallENV15;

pub struct LocalEmbedder {
    /// Inference wants `&mut`, and the trait hands out `&self` because a store is shared. One lock
    /// around the whole model is the honest way to bridge that.
    ///
    /// It serialises every embed, and I am fine with that here: the callers are a search (one
    /// query) and a write (a handful of facts), against a store holding hundreds of rows. If this
    /// ever lands on a hot path, the answer is a small pool of models rather than a finer grained
    /// lock, because the inference session is the thing that cannot be shared, not the map around
    /// it.
    model: Mutex<TextEmbedding>,
}

impl LocalEmbedder {
    /// Load the model: from the build's own copy if it shipped with one, otherwise by fetching it
    /// once into this machine's cache.
    ///
    /// A release bundle carries the model, so recall by meaning works on first launch, offline,
    /// with nothing to wait for and nothing fetched. That is the whole reason to pay the download
    /// size at install time instead: the alternative was every new user's first memory landing
    /// behind a 130MB download, which is the moment they are deciding whether any of this works.
    ///
    /// The fallback is not dead weight. `cargo run` from a source checkout has no bundle and no
    /// resource directory, and asking every contributor to pre-stage a model before the tests will
    /// pass is a worse trade than keeping the path that already worked.
    ///
    /// I return an error rather than panicking or blocking because a machine with no model and no
    /// network still has to be able to use the app: the caller drops back to word matching.
    pub fn new() -> Result<Self, Error> {
        let model = match bundled_dir() {
            Some(dir) => load_bundled(&dir)?,
            None => {
                let options = InitOptions::new(MODEL)
                    .with_cache_dir(cache_dir())
                    .with_show_download_progress(false);
                TextEmbedding::try_new(options).map_err(|e| Error::Model(e.to_string()))?
            }
        };
        Ok(Self {
            model: Mutex::new(model),
        })
    }
}

/// The directory the build put the model in, if there is one and it actually holds a model.
///
/// Checked rather than trusted. A resource directory that exists but is missing a file is a broken
/// build, and the useful behaviour there is to fall through to the download rather than to fail
/// recall entirely: the user gets working memory and the packaging bug shows up as a download that
/// should not have happened.
fn bundled_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os(MODEL_DIR_ENV)?);
    let complete =
        dir.join(ONNX).is_file() && TOKENIZER_FILES.iter().all(|name| dir.join(name).is_file());
    complete.then_some(dir)
}

/// Build the model from files on disk, with no hub client involved at all.
///
/// The pooling and quantization are the ones fastembed itself applies to this model. They are
/// stated here because loading a user-defined model bypasses the table that knows them, and getting
/// either wrong produces embeddings that are silently wrong rather than an error: the search still
/// returns rows, they are just ranked by nothing in particular.
fn load_bundled(dir: &Path) -> Result<TextEmbedding, Error> {
    let read = |name: &str| {
        std::fs::read(dir.join(name))
            .map_err(|e| Error::Model(format!("bundled model file {name}: {e}")))
    };
    let model = UserDefinedEmbeddingModel {
        onnx_file: read(ONNX)?,
        external_initializers: Vec::new(),
        tokenizer_files: TokenizerFiles {
            tokenizer_file: read("tokenizer.json")?,
            config_file: read("config.json")?,
            special_tokens_map_file: read("special_tokens_map.json")?,
            tokenizer_config_file: read("tokenizer_config.json")?,
        },
        // BGE pools on the CLS token, and this is the unquantized model file.
        pooling: Some(Pooling::Cls),
        quantization: QuantizationMode::None,
        output_key: None,
    };
    TextEmbedding::try_new_from_user_defined(model, InitOptionsUserDefined::new())
        .map_err(|e| Error::Model(e.to_string()))
}

impl Embedder for LocalEmbedder {
    fn embed(&self, text: &str) -> Vec<f32> {
        // The trait cannot fail, and I am not going to widen it for a case that means the model is
        // already broken: loading succeeded, so an inference error here is not something the caller
        // can act on. An empty vector scores zero in `cosine`, so the fact sorts last instead of
        // taking the search down with it. A poisoned lock is the same story, a panic in another
        // thread's inference, and the same answer.
        let Ok(mut model) = self.model.lock() else {
            return Vec::new();
        };
        match model.embed(vec![text], None) {
            Ok(mut out) if !out.is_empty() => out.remove(0),
            _ => Vec::new(),
        }
    }
}

/// Where the model file lives. Not the workspace: fastembed defaults to a directory under the
/// current one, which would drop a 130MB blob inside whichever repo the user opened, and do it
/// again for the next repo. One cache per machine, in the place the OS keeps caches.
fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("identra")
        .join("models")
}

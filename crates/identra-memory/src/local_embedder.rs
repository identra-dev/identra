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

use std::path::PathBuf;
use std::sync::Mutex;

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

use crate::{Embedder, Error};

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
    /// Load the model, fetching it once if this machine does not have it yet.
    ///
    /// This is the only part of Identra that talks to the network, and it does it once, for the
    /// model itself, never for a user's memories. After the first run it is a disk read and works
    /// offline. I return an error rather than panicking or blocking because a machine with no
    /// network still has to be able to use the app: the caller drops back to word matching.
    pub fn new() -> Result<Self, Error> {
        let options = InitOptions::new(MODEL)
            .with_cache_dir(cache_dir())
            .with_show_download_progress(false);
        let model = TextEmbedding::try_new(options).map_err(|e| Error::Model(e.to_string()))?;
        Ok(Self {
            model: Mutex::new(model),
        })
    }
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

//! A hands on check that meaning based recall actually answers the question word matching could
//! not. Run it with the model turned on:
//!
//! ```text
//! cargo run -p identra-memory --features fastembed --example recall_check
//! ```
//!
//! The first run fetches the model, so give it a minute. It is not a #[test] because a test that
//! downloads 130MB is a test that fails in CI for reasons that have nothing to do with the code.

#[cfg(not(feature = "fastembed"))]
fn main() {
    eprintln!("run me with --features fastembed");
}

#[cfg(feature = "fastembed")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use identra_memory::{Filter, LocalEmbedder, Scope, Store};
    use std::sync::Arc;

    let store = Store::open_in_memory()?.with_embedder(Arc::new(LocalEmbedder::new()?))?;
    let scope = Scope {
        user_id: "demo".into(),
        agent_id: "workspace".into(),
        run_id: "seed".into(),
    };

    for fact in [
        "The API issues JWT bearer tokens rather than server side sessions, because the mobile \
         client cannot hold cookies.",
        "Identra stores every memory in one SQLite file inside the workspace.",
        "The team dropped redis and uses postgres listen/notify for the job queue.",
        "Node ids are proven with a per node secret, never asserted in a tool argument.",
    ] {
        store.add(&scope, fact)?;
    }

    let filter = Filter {
        user_id: Some("demo".into()),
        ..Default::default()
    };

    // Each question is worded the way a fresh agent would ask it, sharing as little as possible
    // with the fact it should find. That is the whole point: none of these work on words.
    for question in [
        "how do we handle auth",
        "where does the data get saved",
        "what did we decide about the message broker",
        "can an agent pretend to be someone else",
    ] {
        let hits = store.search(&filter, question, 1)?;
        match hits.first() {
            Some(m) => println!("{question}\n  -> {}\n", m.content),
            None => println!("{question}\n  -> NOTHING FOUND\n"),
        }
    }
    Ok(())
}

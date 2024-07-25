//! Nautilus grammar mutator, see <https://github.com/nautilus-fuzz/nautilus>
use alloc::{borrow::Cow, string::String};
use core::{fmt::Debug, marker::PhantomData};
use std::fs::create_dir_all;

use libafl_bolts::{tuples::MatchName, Named};
use serde::{Deserialize, Serialize};

use crate::{
    common::nautilus::grammartec::{chunkstore::ChunkStore, context::Context},
    corpus::{Corpus, HasCorpus, Testcase},
    events::EventFirer,
    executors::ExitKind,
    feedbacks::Feedback,
    generators::NautilusContext,
    inputs::NautilusInput,
    observers::ObserversTuple,
    state::State,
    Error, HasMetadata,
};

/// Metadata for Nautilus grammar mutator chunks
#[derive(Serialize, Deserialize)]
pub struct NautilusChunksMetadata {
    /// the chunk store
    pub cks: ChunkStore,
}

impl Debug for NautilusChunksMetadata {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "NautilusChunksMetadata {{ {} }}",
            serde_json::to_string_pretty(self).unwrap(),
        )
    }
}

libafl_bolts::impl_serdeany!(NautilusChunksMetadata);

impl NautilusChunksMetadata {
    /// Creates a new [`NautilusChunksMetadata`]
    #[must_use]
    pub fn new(work_dir: String) -> Self {
        create_dir_all(format!("{}/outputs/chunks", &work_dir))
            .expect("Could not create folder in workdir");
        Self {
            cks: ChunkStore::new(work_dir),
        }
    }
}

/// A nautilus feedback for grammar fuzzing
pub struct NautilusFeedback<'a> {
    ctx: &'a Context,
}

impl Debug for NautilusFeedback<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NautilusFeedback").finish_non_exhaustive()
    }
}

impl<'a> NautilusFeedback<'a> {
    /// Create a new [`NautilusFeedback`]
    #[must_use]
    pub fn new(context: &'a NautilusContext) -> Self {
        Self { ctx: &context.ctx }
    }
}

impl<'a> Named for NautilusFeedback<'a> {
    fn name(&self) -> &Cow<'static, str> {
        static NAME: Cow<'static, str> = Cow::Borrowed("NautilusFeedback");
        &NAME
    }
}

impl<'a, EM, OT, S> Feedback<EM, NautilusInput, OT, S> for NautilusFeedback<'a>
where
    S: HasMetadata,
{
    #[cfg(feature = "track_hit_feedbacks")]
    fn last_result(&self) -> Result<bool, Error> {
        Ok(false)
    }

    fn append_metadata(
        &mut self,
        state: &mut S,
        _manager: &mut EM,
        _observers: &OT,
        testcase: &mut Testcase<NautilusInput>,
    ) -> Result<(), Error> {
        // TODO is it necessary to clone the whole input here? Maybe we should improve add_tree
        let input = testcase
            .input()
            .as_ref()
            .ok_or_else(|| {
                Error::illegal_state("Testcase presumed to be filled when calling append_metadata")
            })?
            .clone();
        let meta = state
            .metadata_map_mut()
            .get_mut::<NautilusChunksMetadata>()
            .expect("NautilusChunksMetadata not in the state");
        meta.cks.add_tree(input.tree, self.ctx);
        Ok(())
    }
}

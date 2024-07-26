//! Concolic feedback for concolic fuzzing.
//! It is used to attach concolic tracing metadata to the testcase.
//! This feedback should be used in combination with another feedback as this feedback always considers testcases
//! to be not interesting.
//! Requires a [`ConcolicObserver`] to observe the concolic trace.
use alloc::borrow::Cow;
use core::fmt::Debug;

use libafl_bolts::{
    tuples::{Handle, Handled, MatchNameRef},
    Named,
};

use crate::{
    corpus::Testcase, feedbacks::Feedback, observers::concolic::ConcolicObserver, Error,
    HasMetadata,
};

/// The concolic feedback. It is used to attach concolic tracing metadata to the testcase.
/// This feedback should be used in combination with another feedback as this feedback always considers testcases
/// to be not interesting.
/// Requires a [`ConcolicObserver`] to observe the concolic trace.
#[derive(Debug)]
pub struct ConcolicFeedback<'map> {
    observer_handle: Handle<ConcolicObserver<'map>>,
}

impl<'map> ConcolicFeedback<'map> {
    /// Creates a concolic feedback from an observer
    #[allow(unused)]
    #[must_use]
    pub fn from_observer(observer: &ConcolicObserver<'map>) -> Self {
        Self {
            observer_handle: observer.handle(),
        }
    }
}

impl Named for ConcolicFeedback<'_> {
    fn name(&self) -> &Cow<'static, str> {
        self.observer_handle.name()
    }
}

impl<EM, I, OT, S> Feedback<EM, I, OT, S> for ConcolicFeedback<'_> {
    #[cfg(feature = "track_hit_feedbacks")]
    fn last_result(&self) -> Result<bool, Error> {
        Ok(false)
    }

    fn append_metadata(
        &mut self,
        _state: &mut S,
        _manager: &mut EM,
        observers: &OT,
        testcase: &mut Testcase<I>,
    ) -> Result<(), Error> {
        if let Some(metadata) = observers
            .get(&self.observer_handle)
            .map(ConcolicObserver::create_metadata_from_current_map)
        {
            testcase.metadata_map_mut().insert(metadata);
        }
        Ok(())
    }
}

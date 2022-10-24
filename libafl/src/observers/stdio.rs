//! The [`StdOutObserver`] and [`StdErrObserver`] observers look at the stdout of a program
//! The executor must explicitly support these observers.
//! For example, they are supported on the [`crate::executors::CommandExecutor`].

use alloc::string::String;

use serde::{Deserialize, Serialize};

use crate::{bolts::tuples::Named, inputs::UsesInput, observers::Observer};

/// An observer that captures stdout of a target.
/// Only works for supported executors.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StdOutObserver {
    /// The name of the observer.
    pub name: String,
    /// The stdout of the target during its last execution.
    pub stdout: Option<String>,
}

/// An observer that captures stdout of a target.
impl StdOutObserver {
    /// Create a new [`StdOutObserver`] with the given name.
    #[must_use]
    pub fn new(name: String) -> Self {
        Self { name, stdout: None }
    }
}

impl<S> Observer<S> for StdOutObserver
where
    S: UsesInput,
{
    #[inline]
    fn observes_stdout(&mut self) -> bool {
        true
    }
    /// React to new `stdout`
    fn observe_stdout(&mut self, stdout: &str) {
        self.stdout = Some(stdout.into());
    }
}

impl Named for StdOutObserver {
    fn name(&self) -> &str {
        &self.name
    }
}

/// An observer that captures stderr of a target.
/// Only works for supported executors.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StdErrObserver {
    /// The name of the observer.
    pub name: String,
    /// The stderr of the target during its last execution.
    pub stderr: Option<String>,
}

/// An observer that captures stderr of a target.
impl StdErrObserver {
    /// Create a new [`StdErrObserver`] with the given name.
    #[must_use]
    pub fn new(name: String) -> Self {
        Self { name, stderr: None }
    }
}

impl<S> Observer<S> for StdErrObserver
where
    S: UsesInput,
{
    #[inline]
    fn observes_stderr(&mut self) -> bool {
        true
    }

    /// Do nothing on new `stderr`
    fn observe_stderr(&mut self, stderr: &str) {
        self.stderr = Some(stderr.into());
    }
}

impl Named for StdErrObserver {
    fn name(&self) -> &str {
        &self.name
    }
}

//! Docker container execution for SWE-bench test running.
//!
//! Uses Epoch AI's optimized Docker images for SWE-bench evaluation.
//!
//! ## Module layout
//!
//! - [`images`]    — image discovery, pulling, and existence checks
//! - [`container`] — testbed container lifecycle (start/stop, patch application,
//!                   `docker exec` helper)
//! - [`runner`]    — full `run_tests` flow + container wait/log/test-command building
//! - [`parse`]     — pure parsing of pytest/Django test output (no Docker required)

use anyhow::{Context, Result};
use bollard::Docker;

mod container;
mod images;
mod parse;
mod runner;

#[cfg(test)]
mod tests;

/// Default timeout for test execution in seconds.
const DEFAULT_TEST_TIMEOUT_SECS: u64 = 600; // 10 minutes

/// Default timeout for image pull in seconds.
const DEFAULT_PULL_TIMEOUT_SECS: u64 = 300; // 5 minutes

/// Docker executor for running SWE-bench tests.
pub struct DockerExecutor {
    /// Docker client
    pub(in crate::docker) client: Docker,
    /// Test execution timeout in seconds
    pub(in crate::docker) test_timeout_secs: u64,
    /// Image pull timeout in seconds
    pub(in crate::docker) pull_timeout_secs: u64,
}

impl DockerExecutor {
    /// Create a new Docker executor.
    pub fn new() -> Result<Self> {
        let client =
            Docker::connect_with_local_defaults().context("Failed to connect to Docker daemon")?;

        Ok(Self {
            client,
            test_timeout_secs: DEFAULT_TEST_TIMEOUT_SECS,
            pull_timeout_secs: DEFAULT_PULL_TIMEOUT_SECS,
        })
    }

    /// Set the test execution timeout.
    pub fn with_test_timeout(mut self, secs: u64) -> Self {
        self.test_timeout_secs = secs;
        self
    }

    /// Set the image pull timeout.
    pub fn with_pull_timeout(mut self, secs: u64) -> Self {
        self.pull_timeout_secs = secs;
        self
    }
}

impl Default for DockerExecutor {
    fn default() -> Self {
        Self::new().expect("Failed to create default DockerExecutor")
    }
}

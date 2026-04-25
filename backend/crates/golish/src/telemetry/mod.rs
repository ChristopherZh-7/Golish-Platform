//! OpenTelemetry/Langfuse tracing integration for Golish.
//!
//! Split into thematic submodules (stats, counting_processor, filter,
//! langfuse, guard, init).

mod counting_processor;
mod filter;
mod guard;
mod init;
mod langfuse;
mod stats;

pub use guard::TelemetryGuard;
pub use init::init_tracing;
pub use langfuse::LangfuseConfig;
pub use stats::{TelemetryStats, TelemetryStatsSnapshot};

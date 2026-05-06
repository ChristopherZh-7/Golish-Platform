//! Built-in execution mode policies. Add a new mode by creating a new
//! sub-module here, implementing [`super::policy::ExecutionModePolicy`],
//! and registering it in
//! [`super::registry::ExecutionModeRegistry::default`].

pub mod chat;
pub mod task;

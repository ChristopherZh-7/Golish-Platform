//! Memory storage and retrieval methods on [`super::DbTracker`].
//!
//! Split into three thematic submodules:
//! - [`store`]: writers (`store_memory`, `store_memory_global`,
//!   `store_memory_with_*`, `maybe_store_tool_memory`).
//! - [`search`]: keyword / semantic / hybrid search across the `memories`
//!   table plus document-type filtered variants.
//! - [`fetch`]: bulk fetch helpers used by briefings and the recent-memories UI.

mod fetch;
mod search;
mod store;

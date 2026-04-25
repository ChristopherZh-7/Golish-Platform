//! Target & directory-entry data layer for the pentest UI.
//!
//! Split into thematic submodules:
//!
//! - [`types`]: core target DTOs (`Target`, `TargetType`, `Scope`,
//!   `TargetStatus`, `TargetStore`) + the database row adapter.
//! - [`recon`]: `ReconUpdate` extended-scan payload + `DirectoryEntry` /
//!   `DirEntryRow`.
//! - [`db`]: plain database helpers (no Tauri annotations) used by both the
//!   command wrappers and other modules that write through directly.
//! - [`cmds`]: `#[tauri::command]` entry points for target management.
//! - [`directory`]: directory-entry DB helpers + `directory_entry_list`
//!   Tauri command.

mod cmds;
mod db;
mod directory;
mod recon;
mod types;

pub use cmds::*;
pub use db::*;
pub use directory::*;
pub use recon::*;
pub use types::*;

#[doc(hidden)]
pub use cmds::{
    __cmd__target_add, __cmd__target_batch_add, __cmd__target_clear_all, __cmd__target_delete,
    __cmd__target_list, __cmd__target_update, __cmd__target_update_status,
};
#[doc(hidden)]
pub use directory::__cmd__directory_entry_list;

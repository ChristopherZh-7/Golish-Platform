//! File operation tools: `read_file`, `write_file`, `create_file`, `edit_file`,
//! `delete_file`.
//!
//! Each verb lives in its own submodule; shared helpers (binary-file
//! detection) live in [`helpers`].

mod create;
mod delete;
mod edit;
mod helpers;
mod read;
mod write;

#[cfg(test)]
mod tests;

pub use create::CreateFileTool;
pub use delete::DeleteFileTool;
pub use edit::EditFileTool;
pub use read::ReadFileTool;
pub use write::WriteFileTool;

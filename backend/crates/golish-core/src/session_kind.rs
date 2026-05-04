//! Session ID prefix conventions for special session types.
//!
//! Session IDs minted by special internal flows are prefixed so the
//! bridge configurator can short-circuit tool registration / sub-agent
//! loading. Currently the only such flow is the silent title-generation
//! session (a short, tool-less LLM call that summarises a conversation
//! into a 4–6 word title).
//!
//! ## Cross-process invariant
//!
//! Both the Rust backend and the TypeScript frontend mint session IDs
//! using the same prefix. The frontend mirror lives in
//! `frontend/lib/api/ai.ts` (constant `TITLE_GEN_SESSION_PREFIX`). Any
//! change here must be reflected there.

/// Prefix used by title-generation sessions.
///
/// Title-gen sessions disable all tools / sub-agents in
/// `configure_bridge` so the LLM only emits the title text.
pub const TITLE_GEN_SESSION_PREFIX: &str = "title-gen-";

/// Returns `true` if the session id was minted for title generation.
#[inline]
pub fn is_title_gen_session_id(session_id: &str) -> bool {
    session_id.starts_with(TITLE_GEN_SESSION_PREFIX)
}

/// Build a title-generation session id from a base id (typically a
/// conversation id).
#[inline]
pub fn title_gen_session_id(base: &str) -> String {
    format!("{}{}", TITLE_GEN_SESSION_PREFIX, base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_title_gen_session() {
        assert!(is_title_gen_session_id("title-gen-abc"));
        assert!(is_title_gen_session_id(&title_gen_session_id("conv-42")));
        assert!(!is_title_gen_session_id("normal-session-1"));
        assert!(!is_title_gen_session_id(""));
    }

    #[test]
    fn builds_consistent_prefix() {
        let id = title_gen_session_id("conv-42");
        assert_eq!(id, "title-gen-conv-42");
    }
}

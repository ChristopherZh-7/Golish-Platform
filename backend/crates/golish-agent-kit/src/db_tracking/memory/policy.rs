//! Policy boundary between legacy conversational memory and canonical harness facts.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LegacyToolMemoryContext {
    GeneralConversation,
    HarnessCustomerFact,
}

/// Automatic tool-result memory is only a convenience for general
/// conversation. Harness facts must flow through canonical rows + outbox so
/// evidence, invalidation and ownership cannot be bypassed by free-form text.
pub const fn should_store_legacy_tool_memory(context: LegacyToolMemoryContext) -> bool {
    matches!(context, LegacyToolMemoryContext::GeneralConversation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harness_tool_result_never_enters_legacy_memory() {
        assert!(!should_store_legacy_tool_memory(
            LegacyToolMemoryContext::HarnessCustomerFact
        ));
        assert!(should_store_legacy_tool_memory(
            LegacyToolMemoryContext::GeneralConversation
        ));
    }

    #[test]
    fn cutoff_is_confined_to_the_single_automatic_writer() {
        let automatic_store = include_str!("store.rs");
        assert_eq!(
            automatic_store
                .matches("pub fn maybe_store_tool_memory")
                .count(),
            1
        );
        let explicit_tools = include_str!("../../tool_executors/memory.rs");
        assert!(explicit_tools.contains("tracker.store_memory("));
        assert!(explicit_tools.contains("tracker.store_memory_global("));
        assert!(!explicit_tools.contains("should_store_legacy_tool_memory"));
    }
}

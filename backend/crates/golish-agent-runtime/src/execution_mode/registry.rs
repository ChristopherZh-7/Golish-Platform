//! [`ExecutionModeRegistry`] — lookup table of registered policies by id.
//!
//! The runtime asks the registry "give me the policy for `chat` / `task`"
//! per turn. To add a future mode, register a new policy in
//! [`ExecutionModeRegistry::default`] (or call `register` on a custom
//! registry composed at startup).

use std::collections::HashMap;
use std::sync::Arc;

use super::modes::{chat::ChatModePolicy, task::TaskModePolicy};
use super::policy::ExecutionModePolicy;

pub struct ExecutionModeRegistry {
    policies: HashMap<&'static str, Arc<dyn ExecutionModePolicy>>,
}

impl ExecutionModeRegistry {
    pub fn empty() -> Self {
        Self {
            policies: HashMap::new(),
        }
    }

    pub fn register<P: ExecutionModePolicy>(&mut self, policy: P) {
        self.policies.insert(policy.id(), Arc::new(policy));
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn ExecutionModePolicy>> {
        self.policies.get(id).cloned()
    }

    /// All registered policy ids, sorted alphabetically for stable
    /// ordering across IPC / UI calls.
    pub fn list_ids(&self) -> Vec<&'static str> {
        let mut ids: Vec<&'static str> = self.policies.keys().copied().collect();
        ids.sort();
        ids
    }

    pub fn list_all(&self) -> Vec<Arc<dyn ExecutionModePolicy>> {
        let mut all: Vec<Arc<dyn ExecutionModePolicy>> = self.policies.values().cloned().collect();
        all.sort_by_key(|p| p.id());
        all
    }
}

impl Default for ExecutionModeRegistry {
    fn default() -> Self {
        let mut r = Self::empty();
        r.register(ChatModePolicy);
        r.register(TaskModePolicy);
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_registry_has_chat_and_task() {
        let r = ExecutionModeRegistry::default();
        assert!(r.get("chat").is_some());
        assert!(r.get("task").is_some());
        assert_eq!(r.list_ids(), vec!["chat", "task"]);
    }

    #[test]
    fn unknown_mode_returns_none() {
        let r = ExecutionModeRegistry::default();
        assert!(r.get("plan").is_none());
        assert!(r.get("nonexistent").is_none());
    }

    #[test]
    fn list_all_is_alphabetical_by_id() {
        let r = ExecutionModeRegistry::default();
        let ids: Vec<&str> = r.list_all().iter().map(|p| p.id()).collect();
        assert_eq!(ids, vec!["chat", "task"]);
    }

    #[test]
    fn empty_registry_starts_with_no_modes() {
        let r = ExecutionModeRegistry::empty();
        assert!(r.list_ids().is_empty());
        assert!(r.get("chat").is_none());
    }
}

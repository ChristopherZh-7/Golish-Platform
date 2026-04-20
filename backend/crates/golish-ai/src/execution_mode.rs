use serde::{Deserialize, Serialize};

/// Execution mode controls the AI's operational strategy.
///
/// - `Chat`: Single-turn conversational mode (PentAGI's "Assistant").
///   The user sends messages, the AI responds with tools and delegation.
///   No automatic planning or decomposition.
///
/// - `Task`: Full automation mode (PentAGI's "Task").
///   The user submits a goal, and the system automatically:
///   1. Generator decomposes into subtasks
///   2. Primary Agent executes each subtask (with delegation)
///   3. Refiner adjusts the plan after each subtask
///   4. Reporter produces a final summary
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExecutionMode {
    #[default]
    Chat,
    Task,
}

impl ExecutionMode {
    pub fn is_task(&self) -> bool {
        matches!(self, ExecutionMode::Task)
    }

    pub fn is_chat(&self) -> bool {
        matches!(self, ExecutionMode::Chat)
    }
}

impl std::fmt::Display for ExecutionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecutionMode::Chat => write!(f, "chat"),
            ExecutionMode::Task => write!(f, "task"),
        }
    }
}

impl std::str::FromStr for ExecutionMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "chat" => Ok(ExecutionMode::Chat),
            "task" => Ok(ExecutionMode::Task),
            _ => Err(format!("Invalid execution mode: {}", s)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_is_chat() {
        assert_eq!(ExecutionMode::default(), ExecutionMode::Chat);
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", ExecutionMode::Chat), "chat");
        assert_eq!(format!("{}", ExecutionMode::Task), "task");
    }

    #[test]
    fn test_from_str() {
        assert_eq!("chat".parse::<ExecutionMode>().unwrap(), ExecutionMode::Chat);
        assert_eq!("task".parse::<ExecutionMode>().unwrap(), ExecutionMode::Task);
        assert!("invalid".parse::<ExecutionMode>().is_err());
    }

    #[test]
    fn test_serde() {
        let mode = ExecutionMode::Task;
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, "\"task\"");

        let parsed: ExecutionMode = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ExecutionMode::Task);
    }

    #[test]
    fn test_mode_checks() {
        assert!(ExecutionMode::Chat.is_chat());
        assert!(!ExecutionMode::Chat.is_task());
        assert!(ExecutionMode::Task.is_task());
        assert!(!ExecutionMode::Task.is_chat());
    }
}

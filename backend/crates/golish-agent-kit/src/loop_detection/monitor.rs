//! Execution monitor for mentor-based corrective guidance (PentAGI pattern).

/// Threshold for same-tool calls before invoking the execution mentor.
const MENTOR_SAME_TOOL_THRESHOLD: usize = 3;
/// Threshold for total tool calls in a turn before invoking the mentor.
const MENTOR_TOTAL_CALL_THRESHOLD: usize = 15;

/// Execution monitor that tracks tool call patterns to decide when
/// to invoke the mentor for corrective guidance (PentAGI pattern).
///
/// Unlike the `LoopDetector` which blocks repeated calls, the monitor
/// triggers a mentor LLM call that provides strategic advice injected
/// into the tool response.
#[derive(Debug)]
pub struct ExecutionMonitor {
    /// Consecutive calls to the same tool name.
    same_tool_count: usize,
    /// Name of the last tool called.
    last_tool_name: Option<String>,
    /// Total tool calls since last mentor invocation.
    total_since_mentor: usize,
    /// Recent tool call log for mentor context.
    recent_calls: Vec<String>,
}

impl ExecutionMonitor {
    pub fn new() -> Self {
        Self {
            same_tool_count: 0,
            last_tool_name: None,
            total_since_mentor: 0,
            recent_calls: Vec::new(),
        }
    }

    /// Record a tool call and return whether the mentor should be invoked.
    pub fn record_and_check(&mut self, tool_name: &str, args_summary: &str) -> bool {
        self.total_since_mentor += 1;

        if self.last_tool_name.as_deref() == Some(tool_name) {
            self.same_tool_count += 1;
        } else {
            self.same_tool_count = 1;
            self.last_tool_name = Some(tool_name.to_string());
        }

        let entry = format!("{}({})", tool_name, truncate_args(args_summary, 100));
        self.recent_calls.push(entry);
        if self.recent_calls.len() > 10 {
            self.recent_calls.remove(0);
        }

        self.same_tool_count >= MENTOR_SAME_TOOL_THRESHOLD
            || self.total_since_mentor >= MENTOR_TOTAL_CALL_THRESHOLD
    }

    /// Reset counters after a successful mentor intervention.
    pub fn reset_after_mentor(&mut self) {
        self.same_tool_count = 0;
        self.total_since_mentor = 0;
    }

    /// Get the name of the most-repeated tool (for mentor context).
    pub fn repeated_tool_name(&self) -> &str {
        self.last_tool_name.as_deref().unwrap_or("unknown")
    }

    /// Get the same-tool count for mentor context.
    pub fn same_tool_count(&self) -> usize {
        self.same_tool_count
    }

    /// Get recent tool calls formatted for the mentor prompt.
    pub fn recent_calls_summary(&self) -> String {
        self.recent_calls.join("\n")
    }
}

impl Default for ExecutionMonitor {
    fn default() -> Self {
        Self::new()
    }
}

fn truncate_args(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        let mut end = max;
        while !s.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        &s[..end]
    }
}

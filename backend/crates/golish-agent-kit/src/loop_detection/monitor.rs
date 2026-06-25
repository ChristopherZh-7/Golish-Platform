//! Execution monitor trigger for RuntimeSupervisor guidance (PentAGI pattern).

/// Threshold for consecutive equivalent failures before invoking RuntimeSupervisor.
const SUPERVISOR_CONSECUTIVE_FAILURE_THRESHOLD: usize = 3;

/// How RuntimeSupervisor guidance is handled when the monitor triggers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMonitorMode {
    /// Record guidance to logs/trace, but do not inject it into the agent's next
    /// tool response.
    Shadow,
    /// Append RuntimeSupervisor guidance to the tool response.
    SoftInject,
    /// Append a stronger, blocking-style RuntimeSupervisor instruction to the
    /// tool response.
    HardInject,
}

impl ExecutionMonitorMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shadow => "shadow",
            Self::SoftInject => "soft",
            Self::HardInject => "hard",
        }
    }

    pub fn injects(self) -> bool {
        matches!(self, Self::SoftInject | Self::HardInject)
    }
}

/// Execution monitor that tracks failed tool-result patterns to decide when
/// to invoke RuntimeSupervisor for corrective guidance (PentAGI pattern).
///
/// Unlike the `LoopDetector` which blocks repeated calls, the monitor
/// triggers a stage-aware strategy review that may be injected into the tool
/// response after policy sanitization.
#[derive(Debug)]
pub struct ExecutionMonitor {
    mode: ExecutionMonitorMode,
    /// Consecutive equivalent failed results.
    consecutive_failure_count: usize,
    /// Stable key of the last failed tool/result pattern.
    last_failure_key: Option<String>,
    /// Name of the tool involved in the current failed pattern.
    last_failure_tool_name: Option<String>,
    /// Total tool calls since last supervisor invocation.
    total_since_supervisor: usize,
    /// Recent tool call log for supervisor context.
    recent_calls: Vec<String>,
}

impl ExecutionMonitor {
    pub fn new() -> Self {
        Self::soft_inject()
    }

    pub fn shadow() -> Self {
        Self::with_mode(ExecutionMonitorMode::Shadow)
    }

    pub fn soft_inject() -> Self {
        Self::with_mode(ExecutionMonitorMode::SoftInject)
    }

    pub fn hard_inject() -> Self {
        Self::with_mode(ExecutionMonitorMode::HardInject)
    }

    pub fn with_mode(mode: ExecutionMonitorMode) -> Self {
        Self {
            mode,
            consecutive_failure_count: 0,
            last_failure_key: None,
            last_failure_tool_name: None,
            total_since_supervisor: 0,
            recent_calls: Vec::new(),
        }
    }

    pub fn mode(&self) -> ExecutionMonitorMode {
        self.mode
    }

    /// Record a completed tool result and return whether RuntimeSupervisor
    /// should be invoked.
    ///
    /// Normal repeated scans over different targets are expected in pentest
    /// stages, so successful calls never trigger the supervisor. We only
    /// escalate when the same tool/result failure pattern repeats.
    pub fn record_result_and_check(
        &mut self,
        tool_name: &str,
        args_summary: &str,
        success: bool,
        result_summary: &str,
    ) -> bool {
        self.total_since_supervisor += 1;

        let entry = if success {
            format!("{}({}) ok", tool_name, truncate_args(args_summary, 100))
        } else {
            format!(
                "{}({}) fail {}",
                tool_name,
                truncate_args(args_summary, 100),
                failure_signature(result_summary, 120)
            )
        };
        self.recent_calls.push(entry);
        if self.recent_calls.len() > 10 {
            self.recent_calls.remove(0);
        }

        if success {
            self.consecutive_failure_count = 0;
            self.last_failure_key = None;
            self.last_failure_tool_name = None;
            return false;
        }

        let failure_key = format!(
            "{}|{}|{}",
            tool_name,
            truncate_args(args_summary, 180),
            failure_signature(result_summary, 240)
        );
        if self.last_failure_key.as_deref() == Some(failure_key.as_str()) {
            self.consecutive_failure_count += 1;
        } else {
            self.consecutive_failure_count = 1;
            self.last_failure_key = Some(failure_key);
            self.last_failure_tool_name = Some(tool_name.to_string());
        }

        self.consecutive_failure_count >= SUPERVISOR_CONSECUTIVE_FAILURE_THRESHOLD
    }

    /// Reset counters after a successful supervisor intervention.
    pub fn reset_after_supervisor(&mut self) {
        self.consecutive_failure_count = 0;
        self.last_failure_key = None;
        self.last_failure_tool_name = None;
        self.total_since_supervisor = 0;
    }

    /// Get the name of the failed-pattern tool (for supervisor context).
    pub fn repeated_tool_name(&self) -> &str {
        self.last_failure_tool_name.as_deref().unwrap_or("unknown")
    }

    /// Get the consecutive failure count for supervisor context.
    pub fn same_tool_count(&self) -> usize {
        self.consecutive_failure_count
    }

    /// Get recent tool calls formatted for the supervisor prompt.
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

fn failure_signature(s: &str, max: usize) -> &str {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return "(empty failure)";
    }
    truncate_args(trimmed, max)
}

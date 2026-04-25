/// Semantic regions in terminal output based on OSC 133 shell integration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TerminalRegion {
    /// Not in any tracked region - output is passed through
    #[default]
    Output,
    /// Between OSC 133;A and OSC 133;B - prompt text, should be suppressed
    Prompt,
    /// Between OSC 133;B and OSC 133;C - user typing, should be suppressed
    Input,
}

/// Result of parsing terminal output with filtering
#[derive(Debug, Clone)]
pub struct ParseResult {
    /// Extracted semantic events
    pub events: Vec<OscEvent>,
    /// Filtered output bytes - only includes Output region content (not Prompt or Input)
    pub output: Vec<u8>,
}

/// Events extracted from terminal escape sequences (OSC and CSI)
#[derive(Debug, Clone)]
pub enum OscEvent {
    /// OSC 133 ; A - Prompt start
    PromptStart,
    /// OSC 133 ; B - Prompt end (user can type)
    PromptEnd,
    /// OSC 133 ; C [; command] - Command execution started
    CommandStart { command: Option<String> },
    /// OSC 133 ; D ; N - Command finished with exit code N
    CommandEnd { exit_code: i32 },
    /// OSC 7 - Current working directory changed
    DirectoryChanged { path: String },
    /// OSC 1337 ; CurrentDir=PATH ; VirtualEnv=NAME - Virtual environment activated
    /// Reports the virtual environment name when activated (e.g., Python venv, conda)
    VirtualEnvChanged { name: Option<String> },
    /// CSI ? 1049 h (or 47, 1047) - Alternate screen buffer enabled
    /// Indicates a TUI application (vim, htop, less, etc.) has started
    AlternateScreenEnabled,
    /// CSI ? 1049 l (or 47, 1047) - Alternate screen buffer disabled
    /// Indicates a TUI application has exited
    AlternateScreenDisabled,
    /// CSI ? 2026 h - Synchronized output enabled
    /// Applications use this to batch screen updates atomically to prevent flickering
    SynchronizedOutputEnabled,
    /// CSI ? 2026 l - Synchronized output disabled
    /// Signals that batched updates should be flushed to the screen
    SynchronizedOutputDisabled,
    /// CSI ? 1000 h - X10 mouse reporting enabled (button press/release)
    MouseReportingEnabled,
    /// CSI ? 1000 l - Mouse reporting disabled
    MouseReportingDisabled,
    /// CSI ? 1006 h - SGR extended mouse protocol enabled
    SgrMouseEnabled,
    /// CSI ? 1006 l - SGR mouse protocol disabled
    SgrMouseDisabled,
    /// CSI ? 2004 h - Bracketed paste mode enabled
    BracketedPasteEnabled,
    /// CSI ? 2004 l - Bracketed paste mode disabled
    BracketedPasteDisabled,
}

impl OscEvent {
    /// Convert to a tuple of (event_name, CommandBlockEvent) for emission.
    /// Returns None for DirectoryChanged events (handled separately).
    pub fn to_command_block_event(
        &self,
        session_id: &str,
    ) -> Option<(&'static str, crate::manager::CommandBlockEvent)> {
        use crate::manager::CommandBlockEvent;

        Some(match self {
            OscEvent::PromptStart => (
                "command_block",
                CommandBlockEvent {
                    session_id: session_id.to_string(),
                    command: None,
                    exit_code: None,
                    event_type: "prompt_start".to_string(),
                },
            ),
            OscEvent::PromptEnd => (
                "command_block",
                CommandBlockEvent {
                    session_id: session_id.to_string(),
                    command: None,
                    exit_code: None,
                    event_type: "prompt_end".to_string(),
                },
            ),
            OscEvent::CommandStart { command } => (
                "command_block",
                CommandBlockEvent {
                    session_id: session_id.to_string(),
                    command: command.clone(),
                    exit_code: None,
                    event_type: "command_start".to_string(),
                },
            ),
            OscEvent::CommandEnd { exit_code } => (
                "command_block",
                CommandBlockEvent {
                    session_id: session_id.to_string(),
                    command: None,
                    exit_code: Some(*exit_code),
                    event_type: "command_end".to_string(),
                },
            ),
            OscEvent::DirectoryChanged { .. } => return None,
            OscEvent::VirtualEnvChanged { .. } => return None,
            // Alternate screen, synchronized output, mouse, and bracketed paste events
            // are handled separately — they don't map to command block events
            OscEvent::AlternateScreenEnabled
            | OscEvent::AlternateScreenDisabled
            | OscEvent::SynchronizedOutputEnabled
            | OscEvent::SynchronizedOutputDisabled
            | OscEvent::MouseReportingEnabled
            | OscEvent::MouseReportingDisabled
            | OscEvent::SgrMouseEnabled
            | OscEvent::SgrMouseDisabled
            | OscEvent::BracketedPasteEnabled
            | OscEvent::BracketedPasteDisabled => return None,
        })
    }
}

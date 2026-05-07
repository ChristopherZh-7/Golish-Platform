//! UI, window, terminal, and per-agent behavioural settings.
//!
//! Grouped here because these are all surface-level preferences that govern
//! how the user interacts with the app: theme/banner toggles, the persisted
//! window geometry, terminal font/scrollback/caret tweaks, and lightweight
//! agent-runtime knobs (session retention, auto-approval thresholds).

use super::enums::Theme;
use serde::{Deserialize, Serialize};

/// Tool enablement settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolsSettings {
    /// Enable web search tools (Tavily).
    pub web_search: bool,
}

/// User interface preferences.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiSettings {
    /// Theme.
    pub theme: Theme,

    /// Show tips on startup.
    pub show_tips: bool,

    /// Hide banner/welcome message.
    pub hide_banner: bool,

    /// Window state (persisted on close/resize).
    #[serde(default)]
    pub window: WindowSettings,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            show_tips: true,
            hide_banner: false,
            window: WindowSettings::default(),
        }
    }
}

/// Window state settings (persisted across sessions).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowSettings {
    /// Window width in pixels.
    pub width: u32,

    /// Window height in pixels.
    pub height: u32,

    /// Window X position (None = centered).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<i32>,

    /// Window Y position (None = centered).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<i32>,

    /// Whether the window is maximized.
    pub maximized: bool,
}

impl Default for WindowSettings {
    fn default() -> Self {
        Self {
            width: 1400,
            height: 900,
            x: None,
            y: None,
            maximized: false,
        }
    }
}

/// Caret (text cursor) customization for the input area.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CaretSettings {
    /// Caret style: `"block"` or `"default"` (native browser caret).
    pub style: String,

    /// Block caret width in `ch` units (0.5-3.0).
    pub width: f64,

    /// Caret color as hex string (e.g. `"#FFFFFF"`). None = inherit from theme foreground.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,

    /// Blink speed in milliseconds (0 = no blink).
    pub blink_speed: f64,

    /// Caret opacity (0.0-1.0).
    pub opacity: f64,
}

impl Default for CaretSettings {
    fn default() -> Self {
        Self {
            style: "default".to_string(),
            width: 1.0,
            color: None,
            blink_speed: 530.0,
            opacity: 1.0,
        }
    }
}

/// Terminal configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalSettings {
    /// Default shell override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,

    /// Font family.
    pub font_family: String,

    /// Font size in pixels.
    pub font_size: u32,

    /// Scrollback buffer lines.
    pub scrollback: u32,

    /// Additional commands that trigger fullterm mode.
    /// These are merged with the built-in defaults (claude, cc, codex, etc.).
    /// Most TUI apps are auto-detected via ANSI sequences; this is for edge cases.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fullterm_commands: Vec<String>,

    /// Input caret customization.
    #[serde(default)]
    pub caret: CaretSettings,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            shell: None,
            font_family: "SF Mono".to_string(),
            font_size: 14,
            scrollback: 10000,
            fullterm_commands: Vec::new(),
            caret: CaretSettings::default(),
        }
    }
}

/// Agent behavior settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentSettings {
    /// Auto-save conversations.
    pub session_persistence: bool,

    /// Session retention in days (0 = forever).
    pub session_retention_days: u32,

    /// Enable pattern learning for auto-approval.
    pub pattern_learning: bool,

    /// Minimum approvals before auto-approve.
    pub min_approvals_for_auto: u32,

    /// Approval rate threshold (0.0 - 1.0).
    pub approval_threshold: f64,
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            session_persistence: true,
            session_retention_days: 30,
            pattern_learning: true,
            min_approvals_for_auto: 3,
            approval_threshold: 0.8,
        }
    }
}

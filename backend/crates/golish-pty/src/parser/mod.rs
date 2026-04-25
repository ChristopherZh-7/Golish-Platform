#![allow(dead_code)] // PTY parser implemented but integrated via Tauri feature only

mod performer;
mod types;
#[cfg(test)]
mod tests;

use vte::Parser;

use performer::OscPerformer;

pub use types::{OscEvent, ParseResult};
pub type TerminalRegion = types::TerminalRegion;

/// Terminal output parser that extracts OSC sequences
pub struct TerminalParser {
    parser: Parser,
    pub(in crate::parser) performer: OscPerformer,
}

impl TerminalParser {
    pub fn new() -> Self {
        Self {
            parser: Parser::new(),
            performer: OscPerformer::new(),
        }
    }

    /// Parse terminal output and extract OSC events
    pub fn parse(&mut self, data: &[u8]) -> Vec<OscEvent> {
        self.performer.events.clear();
        for byte in data {
            self.parser.advance(&mut self.performer, *byte);
        }
        std::mem::take(&mut self.performer.events)
    }

    /// Parse terminal output, extract OSC events, and filter output to only include
    /// content from the Output region (excludes Prompt and Input regions).
    ///
    /// When in alternate screen mode (TUI apps like vim, htop), filtering is disabled
    /// and all raw bytes are passed through to preserve escape sequences needed for
    /// proper rendering.
    pub fn parse_filtered(&mut self, data: &[u8]) -> ParseResult {
        // If already in alternate screen mode, pass through raw data
        // TUI apps need all escape sequences for proper rendering
        let was_in_alternate = self.performer.alternate_screen_active;

        self.performer.events.clear();
        self.performer.visible_bytes.clear();

        for byte in data {
            self.parser.advance(&mut self.performer, *byte);
        }

        // If we were in alternate screen OR just entered it, use raw output
        // This ensures TUI apps get all their escape sequences
        let use_raw_output = was_in_alternate || self.performer.alternate_screen_active;

        ParseResult {
            events: std::mem::take(&mut self.performer.events),
            output: if use_raw_output {
                data.to_vec()
            } else {
                std::mem::take(&mut self.performer.visible_bytes)
            },
        }
    }

    /// Check if the parser is currently tracking alternate screen mode as active
    pub fn in_alternate_screen(&self) -> bool {
        self.performer.alternate_screen_active
    }
}

impl Default for TerminalParser {
    fn default() -> Self {
        Self::new()
    }
}

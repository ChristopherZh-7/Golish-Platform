use vte::Params;
use vte::Perform;

use super::types::{OscEvent, TerminalRegion};

pub(in crate::parser) struct OscPerformer {
    pub(in crate::parser) events: Vec<OscEvent>,
    /// Track last directory to deduplicate OSC 7 events
    last_directory: Option<String>,
    /// Track last virtual environment to deduplicate OSC 1337 events
    last_virtual_env: Option<String>,
    /// Current semantic region based on OSC 133 markers
    pub(in crate::parser) current_region: TerminalRegion,
    /// Accumulated visible output bytes (only from Output region)
    pub(in crate::parser) visible_bytes: Vec<u8>,
    /// Track alternate screen state to deduplicate CSI events
    pub(in crate::parser) alternate_screen_active: bool,
}

impl OscPerformer {
    pub(in crate::parser) fn new() -> Self {
        Self {
            events: Vec::new(),
            last_directory: None,
            last_virtual_env: None,
            current_region: TerminalRegion::Output,
            visible_bytes: Vec::new(),
            alternate_screen_active: false,
        }
    }

    /// Reconstruct any CSI sequence into `visible_bytes`.
    ///
    /// Wire format: ESC [ + intermediates + params (;-separated, :-separated for subparams) + action.
    /// This generalises the old SGR-only reconstruction to cover cursor movement, erase,
    /// DEC private modes, and every other CSI sequence xterm.js needs to render correctly.
    fn write_csi_to_visible_bytes(
        &mut self,
        params: &vte::Params,
        intermediates: &[u8],
        action: char,
    ) {
        self.visible_bytes.extend_from_slice(b"\x1b[");
        for &b in intermediates {
            self.visible_bytes.push(b);
        }
        let mut first = true;
        for param in params {
            for (i, &subparam) in param.iter().enumerate() {
                if !first || i > 0 {
                    self.visible_bytes.push(if i > 0 { b':' } else { b';' });
                }
                let mut num_buf = itoa::Buffer::new();
                self.visible_bytes
                    .extend_from_slice(num_buf.format(subparam).as_bytes());
                first = false;
            }
        }
        let mut buf = [0u8; 4];
        self.visible_bytes
            .extend_from_slice(action.encode_utf8(&mut buf).as_bytes());
    }

    fn handle_osc(&mut self, params: &[&[u8]]) {
        if params.is_empty() {
            return;
        }

        // Parse the OSC command number
        let cmd = match std::str::from_utf8(params[0]) {
            Ok(s) => s,
            Err(_) => return,
        };

        match cmd {
            // OSC 133 - Semantic prompt sequences
            "133" => self.handle_osc_133(params),
            // OSC 7 - Current working directory
            "7" => self.handle_osc_7(params),
            // OSC 1337 - Custom data (virtual environment)
            "1337" => self.handle_osc_1337(params),
            _ => {}
        }
    }

    fn handle_osc_133(&mut self, params: &[&[u8]]) {
        if params.len() < 2 {
            tracing::trace!("[OSC 133] Received but params.len() < 2");
            return;
        }

        let marker = match std::str::from_utf8(params[1]) {
            Ok(s) => s,
            Err(_) => {
                tracing::trace!("[OSC 133] Marker is not valid UTF-8");
                return;
            }
        };

        tracing::trace!("[OSC 133] marker={:?}, params_len={}", marker, params.len());

        // Get extra argument from params[2] if present
        let extra_arg = params.get(2).and_then(|p| std::str::from_utf8(p).ok());

        // Match on first character, handling both "C" and "C;command" formats
        match marker.chars().next() {
            Some('A') => {
                tracing::trace!("[OSC 133] PromptStart");
                self.current_region = TerminalRegion::Prompt;
                self.events.push(OscEvent::PromptStart);
            }
            Some('B') => {
                tracing::trace!("[OSC 133] PromptEnd");
                self.current_region = TerminalRegion::Input;
                self.events.push(OscEvent::PromptEnd);
            }
            Some('C') => {
                // Command may come from marker suffix (C;cmd) or params[2]
                self.current_region = TerminalRegion::Output;
                let command = marker
                    .strip_prefix("C;")
                    .or(extra_arg)
                    .map(|s| s.to_string());
                tracing::trace!("[OSC 133] CommandStart: {:?}", command);
                self.events.push(OscEvent::CommandStart { command });
            }
            Some('D') => {
                // Exit code may come from marker suffix (D;0) or params[2]
                // Switch to Prompt so post-command shell artifacts (PROMPT_SP,
                // bracketed-paste toggle, OSC 7, etc.) between D and the next A
                // are NOT captured as command output.
                self.current_region = TerminalRegion::Prompt;
                let exit_code = marker
                    .strip_prefix("D;")
                    .or(extra_arg)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                tracing::trace!("[OSC 133] CommandEnd: exit_code={}", exit_code);
                self.events.push(OscEvent::CommandEnd { exit_code });
            }
            _ => {}
        }
    }

    fn handle_osc_7(&mut self, params: &[&[u8]]) {
        // OSC 7 format: file://hostname/path
        if params.len() < 2 {
            tracing::trace!("[cwd-sync] OSC 7 received but params.len() < 2");
            return;
        }

        let url = match std::str::from_utf8(params[1]) {
            Ok(s) => s,
            Err(_) => {
                tracing::trace!("[cwd-sync] OSC 7 URL is not valid UTF-8");
                return;
            }
        };

        tracing::trace!("[cwd-sync] OSC 7 URL: {}", url);

        // Parse file:// URL
        if let Some(path) = url.strip_prefix("file://") {
            // Remove hostname (everything up to the next /)
            if let Some(idx) = path.find('/') {
                let path = &path[idx..];
                // URL decode the path
                let path = urlencoding_decode(path);

                // Only emit if directory actually changed
                let is_duplicate = self
                    .last_directory
                    .as_ref()
                    .map(|last| last == &path)
                    .unwrap_or(false);

                if is_duplicate {
                    tracing::trace!("[cwd-sync] Duplicate OSC 7 ignored: {}", path);
                } else {
                    tracing::trace!(
                        "[cwd-sync] Directory changed: prev={:?}, new={}",
                        self.last_directory,
                        path
                    );
                    self.last_directory = Some(path.clone());
                    self.events.push(OscEvent::DirectoryChanged { path });
                }
            } else {
                tracing::trace!("[cwd-sync] OSC 7 path has no slash after hostname");
            }
        } else {
            tracing::trace!("[cwd-sync] OSC 7 URL does not start with file://");
        }
    }

    fn handle_osc_1337(&mut self, params: &[&[u8]]) {
        // OSC 1337 format: VirtualEnv=name or just name
        if params.len() < 2 {
            tracing::trace!("[venv-sync] OSC 1337 received but params.len() < 2");
            return;
        }

        let data = match std::str::from_utf8(params[1]) {
            Ok(s) => s,
            Err(_) => {
                tracing::trace!("[venv-sync] OSC 1337 data is not valid UTF-8");
                return;
            }
        };

        tracing::trace!("[venv-sync] OSC 1337 data: {}", data);

        // Parse VirtualEnv=name format, or just use the whole string
        let venv_name = if let Some(name) = data.strip_prefix("VirtualEnv=") {
            if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            }
        } else if data.is_empty() {
            None
        } else {
            Some(data.to_string())
        };

        // Only emit if virtual env actually changed
        let is_duplicate = self
            .last_virtual_env
            .as_ref()
            .map(|last| Some(last) == venv_name.as_ref())
            .unwrap_or(venv_name.is_none());

        if is_duplicate {
            tracing::trace!("[venv-sync] Duplicate OSC 1337 ignored: {:?}", venv_name);
        } else {
            tracing::info!(
                "[venv-sync] Virtual environment changed to: {:?}",
                venv_name
            );
            self.last_virtual_env.clone_from(&venv_name);
            self.events
                .push(OscEvent::VirtualEnvChanged { name: venv_name });
        }
    }
}

impl Perform for OscPerformer {
    fn print(&mut self, c: char) {
        if self.current_region == TerminalRegion::Output {
            // Encode char as UTF-8 and add to visible_bytes
            let mut buf = [0u8; 4];
            let encoded = c.encode_utf8(&mut buf);
            self.visible_bytes.extend_from_slice(encoded.as_bytes());
        }
    }

    fn execute(&mut self, byte: u8) {
        if self.current_region == TerminalRegion::Output {
            // Pass through control characters in Output region
            // Common ones: LF (0x0A), CR (0x0D), TAB (0x09), BS (0x08)
            match byte {
                0x0A | 0x0D | 0x09 | 0x08 => {
                    self.visible_bytes.push(byte);
                }
                _ => {}
            }
        }
    }

    fn hook(&mut self, _params: &Params, _intermediates: &[u8], _ignore: bool, _action: char) {}
    fn put(&mut self, _byte: u8) {}
    fn unhook(&mut self) {}

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        // Pass ESC sequences through to xterm.js in the Output region.
        // Previously this was a no-op, silently dropping sequences like ESC= (DECKPAM,
        // sent by vim) and ESC(B (DEC charset designation).
        if self.current_region == TerminalRegion::Output {
            self.visible_bytes.push(b'\x1b');
            for &b in intermediates {
                self.visible_bytes.push(b);
            }
            self.visible_bytes.push(byte);
        }
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
        // Pass ALL CSI sequences through in the Output region (not just SGR).
        // Previously only action == 'm' was reconstructed; cursor movement (A/B/C/D/H),
        // erase (J/K), and DEC private modes (?1000h, ?1006h, ?2004h) were silently
        // dropped, so the live xterm.js terminal never received them in timeline mode.
        if self.current_region == TerminalRegion::Output {
            self.write_csi_to_visible_bytes(params, intermediates, action);
        }

        // Semantic event emission for DEC private modes (regardless of region).
        if intermediates != [b'?'] {
            return;
        }

        let is_enable = match action {
            'h' => true,  // DECSET - enable mode
            'l' => false, // DECRST - disable mode
            _ => return,
        };

        for param in params {
            let mode = param.first().copied().unwrap_or(0);

            match mode {
                // 1049: xterm alternate screen with saved cursor (most common)
                // 47: legacy alternate screen
                // 1047: alternate screen without cursor save
                1049 | 47 | 1047 => {
                    // Deduplicate: only emit if state actually changes
                    if is_enable && !self.alternate_screen_active {
                        self.alternate_screen_active = true;
                        self.events.push(OscEvent::AlternateScreenEnabled);
                    } else if !is_enable && self.alternate_screen_active {
                        self.alternate_screen_active = false;
                        self.events.push(OscEvent::AlternateScreenDisabled);
                    }
                }
                // 2026: Synchronized output (DEC private mode)
                // Used by modern CLI apps to batch screen updates atomically
                2026 => {
                    if is_enable {
                        self.events.push(OscEvent::SynchronizedOutputEnabled);
                    } else {
                        self.events.push(OscEvent::SynchronizedOutputDisabled);
                    }
                }
                // 1000: X10 mouse reporting (button press/release events)
                1000 => self.events.push(if is_enable {
                    OscEvent::MouseReportingEnabled
                } else {
                    OscEvent::MouseReportingDisabled
                }),
                // 1006: SGR extended mouse protocol (coordinates > 223)
                1006 => self.events.push(if is_enable {
                    OscEvent::SgrMouseEnabled
                } else {
                    OscEvent::SgrMouseDisabled
                }),
                // 2004: Bracketed paste mode (wraps pasted text in ESC[?2004h...ESC[?2004l)
                2004 => self.events.push(if is_enable {
                    OscEvent::BracketedPasteEnabled
                } else {
                    OscEvent::BracketedPasteDisabled
                }),
                _ => {}
            }
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        self.handle_osc(params);
    }
}

/// Simple URL decoding for paths
pub(in crate::parser) fn urlencoding_decode(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '%' {
            let mut hex = String::new();
            if let Some(&h1) = chars.peek() {
                hex.push(h1);
                chars.next();
            }
            if let Some(&h2) = chars.peek() {
                hex.push(h2);
                chars.next();
            }
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                result.push(byte as char);
            } else {
                result.push('%');
                result.push_str(&hex);
            }
        } else {
            result.push(c);
        }
    }

    result
}

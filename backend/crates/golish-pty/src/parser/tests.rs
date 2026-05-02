use super::performer::urlencoding_decode;
use super::*;

// ===========================================
// OSC 133 - Prompt lifecycle tests
// ===========================================

#[test]
fn test_osc_133_prompt_start() {
    let mut parser = TerminalParser::new();
    // OSC 133 ; A ST (using BEL as terminator)
    let data = b"\x1b]133;A\x07";
    let events = parser.parse(data);
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], OscEvent::PromptStart));
}

#[test]
fn test_osc_133_prompt_end() {
    let mut parser = TerminalParser::new();
    // OSC 133 ; B ST (using BEL as terminator)
    let data = b"\x1b]133;B\x07";
    let events = parser.parse(data);
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], OscEvent::PromptEnd));
}

#[test]
fn test_osc_133_prompt_start_with_st_terminator() {
    let mut parser = TerminalParser::new();
    // OSC 133 ; A ST (using ESC \ as string terminator)
    let data = b"\x1b]133;A\x1b\\";
    let events = parser.parse(data);
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], OscEvent::PromptStart));
}

// ===========================================
// OSC 133 - Command lifecycle tests
// ===========================================

#[test]
fn test_osc_133_command_start_no_command() {
    let mut parser = TerminalParser::new();
    // OSC 133 ; C ST (no command text)
    let data = b"\x1b]133;C\x07";
    let events = parser.parse(data);
    assert_eq!(events.len(), 1);
    if let OscEvent::CommandStart { command } = &events[0] {
        assert!(command.is_none());
    } else {
        panic!("Expected CommandStart");
    }
}

#[test]
fn test_osc_133_command_with_text() {
    let mut parser = TerminalParser::new();
    // OSC 133 ; C ; ls -la ST
    let data = b"\x1b]133;C;ls -la\x07";
    let events = parser.parse(data);
    assert_eq!(events.len(), 1);
    if let OscEvent::CommandStart { command } = &events[0] {
        assert_eq!(command.as_deref(), Some("ls -la"));
    } else {
        panic!("Expected CommandStart");
    }
}

#[test]
fn test_osc_133_command_with_complex_text() {
    let mut parser = TerminalParser::new();
    // Complex command with pipes, flags, etc.
    let data = b"\x1b]133;C;cat file.txt | grep -E 'pattern' | head -n 10\x07";
    let events = parser.parse(data);
    assert_eq!(events.len(), 1);
    if let OscEvent::CommandStart { command } = &events[0] {
        assert_eq!(
            command.as_deref(),
            Some("cat file.txt | grep -E 'pattern' | head -n 10")
        );
    } else {
        panic!("Expected CommandStart");
    }
}

#[test]
fn test_osc_133_command_end_success() {
    let mut parser = TerminalParser::new();
    // OSC 133 ; D ; 0 ST
    let data = b"\x1b]133;D;0\x07";
    let events = parser.parse(data);
    assert_eq!(events.len(), 1);
    if let OscEvent::CommandEnd { exit_code } = &events[0] {
        assert_eq!(*exit_code, 0);
    } else {
        panic!("Expected CommandEnd");
    }
}

#[test]
fn test_osc_133_command_end_failure() {
    let mut parser = TerminalParser::new();
    // OSC 133 ; D ; 1 ST (command failed)
    let data = b"\x1b]133;D;1\x07";
    let events = parser.parse(data);
    assert_eq!(events.len(), 1);
    if let OscEvent::CommandEnd { exit_code } = &events[0] {
        assert_eq!(*exit_code, 1);
    } else {
        panic!("Expected CommandEnd");
    }
}

#[test]
fn test_osc_133_command_end_signal() {
    let mut parser = TerminalParser::new();
    // OSC 133 ; D ; 130 ST (Ctrl+C typically gives 128 + 2 = 130)
    let data = b"\x1b]133;D;130\x07";
    let events = parser.parse(data);
    assert_eq!(events.len(), 1);
    if let OscEvent::CommandEnd { exit_code } = &events[0] {
        assert_eq!(*exit_code, 130);
    } else {
        panic!("Expected CommandEnd");
    }
}

#[test]
fn test_osc_133_command_end_no_exit_code() {
    let mut parser = TerminalParser::new();
    // OSC 133 ; D ST (no exit code, defaults to 0)
    let data = b"\x1b]133;D\x07";
    let events = parser.parse(data);
    assert_eq!(events.len(), 1);
    if let OscEvent::CommandEnd { exit_code } = &events[0] {
        assert_eq!(*exit_code, 0);
    } else {
        panic!("Expected CommandEnd");
    }
}

// ===========================================
// Full command lifecycle test
// ===========================================

#[test]
fn test_full_command_lifecycle() {
    let mut parser = TerminalParser::new();

    // Simulate a full command lifecycle:
    // 1. Prompt starts
    // 2. Prompt ends (user can type)
    // 3. Command starts (user pressed enter)
    // 4. Command ends with exit code

    let prompt_start = b"\x1b]133;A\x07";
    let prompt_end = b"\x1b]133;B\x07";
    let command_start = b"\x1b]133;C;echo hello\x07";
    let command_end = b"\x1b]133;D;0\x07";

    let events = parser.parse(prompt_start);
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], OscEvent::PromptStart));

    let events = parser.parse(prompt_end);
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], OscEvent::PromptEnd));

    let events = parser.parse(command_start);
    assert_eq!(events.len(), 1);
    if let OscEvent::CommandStart { command } = &events[0] {
        assert_eq!(command.as_deref(), Some("echo hello"));
    } else {
        panic!("Expected CommandStart");
    }

    let events = parser.parse(command_end);
    assert_eq!(events.len(), 1);
    if let OscEvent::CommandEnd { exit_code } = &events[0] {
        assert_eq!(*exit_code, 0);
    } else {
        panic!("Expected CommandEnd");
    }
}

#[test]
fn test_multiple_events_in_single_parse() {
    let mut parser = TerminalParser::new();
    // Multiple OSC sequences in one chunk
    let data = b"\x1b]133;A\x07\x1b]133;B\x07";
    let events = parser.parse(data);
    assert_eq!(events.len(), 2);
    assert!(matches!(events[0], OscEvent::PromptStart));
    assert!(matches!(events[1], OscEvent::PromptEnd));
}

// ===========================================
// OSC 7 - Directory change tests
// ===========================================

#[test]
fn test_osc_7_directory() {
    let mut parser = TerminalParser::new();
    // OSC 7 ; file://hostname/Users/test ST
    let data = b"\x1b]7;file://localhost/Users/test\x07";
    let events = parser.parse(data);
    assert_eq!(events.len(), 1);
    if let OscEvent::DirectoryChanged { path } = &events[0] {
        assert_eq!(path, "/Users/test");
    } else {
        panic!("Expected DirectoryChanged");
    }
}

#[test]
fn test_osc_7_directory_with_spaces() {
    let mut parser = TerminalParser::new();
    // Path with URL-encoded spaces (%20)
    let data = b"\x1b]7;file://localhost/Users/test/My%20Documents\x07";
    let events = parser.parse(data);
    assert_eq!(events.len(), 1);
    if let OscEvent::DirectoryChanged { path } = &events[0] {
        assert_eq!(path, "/Users/test/My Documents");
    } else {
        panic!("Expected DirectoryChanged");
    }
}

#[test]
fn test_osc_7_directory_deep_path() {
    let mut parser = TerminalParser::new();
    let data = b"\x1b]7;file://macbook.local/Users/xlyk/Code/golish/src-tauri\x07";
    let events = parser.parse(data);
    assert_eq!(events.len(), 1);
    if let OscEvent::DirectoryChanged { path } = &events[0] {
        assert_eq!(path, "/Users/xlyk/Code/golish/src-tauri");
    } else {
        panic!("Expected DirectoryChanged");
    }
}

// ===========================================
// URL encoding/decoding tests
// ===========================================

#[test]
fn test_urlencoding_decode_simple() {
    assert_eq!(urlencoding_decode("/path/to/file"), "/path/to/file");
}

#[test]
fn test_urlencoding_decode_space() {
    assert_eq!(
        urlencoding_decode("/path/My%20Documents"),
        "/path/My Documents"
    );
}

#[test]
fn test_urlencoding_decode_multiple_encoded() {
    assert_eq!(
        urlencoding_decode("/path%20with%20multiple%20spaces"),
        "/path with multiple spaces"
    );
}

#[test]
fn test_urlencoding_decode_special_chars() {
    // %23 = #, %26 = &, %3D = =
    assert_eq!(urlencoding_decode("/path%23file"), "/path#file");
}

#[test]
fn test_urlencoding_decode_invalid_hex() {
    // Invalid hex sequence should be preserved
    assert_eq!(urlencoding_decode("/path%ZZ"), "/path%ZZ");
}

#[test]
fn test_urlencoding_decode_incomplete_percent() {
    // Incomplete percent encoding at end - only 1 hex digit
    // Current implementation tries to decode anyway (0x02 = STX control char)
    assert_eq!(urlencoding_decode("/path%2"), "/path\u{2}");
}

// ===========================================
// Edge cases and robustness tests
// ===========================================

#[test]
fn test_parser_ignores_regular_text() {
    let mut parser = TerminalParser::new();
    // Regular terminal output with no OSC sequences
    let data = b"Hello, world!\nThis is normal output.\n";
    let events = parser.parse(data);
    assert_eq!(events.len(), 0);
}

#[test]
fn test_parser_handles_mixed_content() {
    let mut parser = TerminalParser::new();
    // Normal text mixed with OSC sequences
    let data = b"Some output\x1b]133;A\x07more output\x1b]133;B\x07";
    let events = parser.parse(data);
    assert_eq!(events.len(), 2);
    assert!(matches!(events[0], OscEvent::PromptStart));
    assert!(matches!(events[1], OscEvent::PromptEnd));
}

#[test]
fn test_parser_handles_ansi_escape_codes() {
    let mut parser = TerminalParser::new();
    // ANSI color codes should be ignored, OSC should be parsed
    let data = b"\x1b[32mgreen text\x1b[0m\x1b]133;A\x07";
    let events = parser.parse(data);
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], OscEvent::PromptStart));
}

#[test]
fn test_parser_ignores_unknown_osc() {
    let mut parser = TerminalParser::new();
    // OSC 0 (window title) - should be ignored
    let data = b"\x1b]0;Window Title\x07";
    let events = parser.parse(data);
    assert_eq!(events.len(), 0);
}

#[test]
fn test_parser_empty_input() {
    let mut parser = TerminalParser::new();
    let events = parser.parse(b"");
    assert_eq!(events.len(), 0);
}

#[test]
fn test_parser_partial_osc_sequence() {
    let mut parser = TerminalParser::new();
    // Incomplete OSC sequence (no terminator)
    let data = b"\x1b]133;A";
    let events = parser.parse(data);
    // VTE parser buffers incomplete sequences, so nothing should be emitted yet
    assert_eq!(events.len(), 0);
}

#[test]
fn test_parser_is_stateless_between_calls() {
    let mut parser = TerminalParser::new();

    // First parse
    let events1 = parser.parse(b"\x1b]133;A\x07");
    assert_eq!(events1.len(), 1);

    // Second parse - events from first call should be cleared
    let events2 = parser.parse(b"\x1b]133;B\x07");
    assert_eq!(events2.len(), 1);
    assert!(matches!(events2[0], OscEvent::PromptEnd));
}

#[test]
fn test_parser_default_trait() {
    let mut parser = TerminalParser::default();
    assert!(parser.parse(b"\x1b]133;A\x07").len() == 1);
}

// ===========================================
// Alternate Screen Buffer tests (CSI sequences)
// ===========================================

mod terminal_modes;
mod virtualenv_and_filtered_output;

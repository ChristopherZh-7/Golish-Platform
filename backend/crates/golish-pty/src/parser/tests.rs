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

#[test]
fn test_alternate_screen_enable_1049() {
    let mut parser = TerminalParser::new();
    // ESC [ ? 1049 h - xterm-style alternate screen with saved cursor
    let data = b"\x1b[?1049h";
    let events = parser.parse(data);
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], OscEvent::AlternateScreenEnabled));
}

#[test]
fn test_alternate_screen_disable_1049() {
    let mut parser = TerminalParser::new();
    // First enable, then disable
    parser.parse(b"\x1b[?1049h");
    let events = parser.parse(b"\x1b[?1049l");
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], OscEvent::AlternateScreenDisabled));
}

#[test]
fn test_alternate_screen_enable_47() {
    let mut parser = TerminalParser::new();
    // ESC [ ? 47 h - legacy alternate screen
    let data = b"\x1b[?47h";
    let events = parser.parse(data);
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], OscEvent::AlternateScreenEnabled));
}

#[test]
fn test_alternate_screen_enable_1047() {
    let mut parser = TerminalParser::new();
    // ESC [ ? 1047 h - alternate screen without cursor save
    let data = b"\x1b[?1047h";
    let events = parser.parse(data);
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], OscEvent::AlternateScreenEnabled));
}

#[test]
fn test_alternate_screen_deduplication_enable() {
    let mut parser = TerminalParser::new();
    // Enable twice - should only emit once
    let events1 = parser.parse(b"\x1b[?1049h");
    assert_eq!(events1.len(), 1);

    let events2 = parser.parse(b"\x1b[?1049h");
    assert_eq!(events2.len(), 0); // Deduplicated
}

#[test]
fn test_alternate_screen_deduplication_disable() {
    let mut parser = TerminalParser::new();
    // Disable without prior enable - should not emit
    let events = parser.parse(b"\x1b[?1049l");
    assert_eq!(events.len(), 0);
}

#[test]
fn test_alternate_screen_full_cycle() {
    let mut parser = TerminalParser::new();
    // Full cycle: enable -> disable
    let enable_events = parser.parse(b"\x1b[?1049h");
    assert_eq!(enable_events.len(), 1);
    assert!(matches!(enable_events[0], OscEvent::AlternateScreenEnabled));

    let disable_events = parser.parse(b"\x1b[?1049l");
    assert_eq!(disable_events.len(), 1);
    assert!(matches!(
        disable_events[0],
        OscEvent::AlternateScreenDisabled
    ));
}

#[test]
fn test_alternate_screen_mixed_with_osc() {
    let mut parser = TerminalParser::new();
    // OSC 133 A (prompt start) + CSI ? 1049 h (alt screen)
    let data = b"\x1b]133;A\x07\x1b[?1049h";
    let events = parser.parse(data);
    assert_eq!(events.len(), 2);
    assert!(matches!(events[0], OscEvent::PromptStart));
    assert!(matches!(events[1], OscEvent::AlternateScreenEnabled));
}

#[test]
fn test_non_dec_private_mode_ignored() {
    let mut parser = TerminalParser::new();
    // Standard CSI (no ?) should be ignored - this is not a DEC private mode
    let data = b"\x1b[1049h";
    let events = parser.parse(data);
    assert_eq!(events.len(), 0);
}

#[test]
fn test_alternate_screen_other_modes_ignored() {
    let mut parser = TerminalParser::new();
    // Other DEC private modes should be ignored (e.g., mode 1 for application cursor)
    let data = b"\x1b[?1h";
    let events = parser.parse(data);
    assert_eq!(events.len(), 0);
}

#[test]
fn test_vim_like_startup_sequence() {
    let mut parser = TerminalParser::new();
    // Simulate vim-like startup: various CSI sequences including alt screen
    // Real vim sends more, but this tests the key part
    let data = b"\x1b[?1049h\x1b[22;0;0t\x1b[?1h\x1b=";
    let events = parser.parse(data);
    // Only the alternate screen event should be captured
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], OscEvent::AlternateScreenEnabled));
}

#[test]
fn test_vim_like_exit_sequence() {
    let mut parser = TerminalParser::new();
    // First enter alternate screen
    parser.parse(b"\x1b[?1049h");
    // Simulate vim-like exit
    let data = b"\x1b[?1049l\x1b[23;0;0t\x1b[?1l\x1b>";
    let events = parser.parse(data);
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], OscEvent::AlternateScreenDisabled));
}

// ===========================================
// Synchronized Output (DEC 2026) tests
// ===========================================

#[test]
fn test_synchronized_output_enable() {
    let mut parser = TerminalParser::new();
    // ESC [ ? 2026 h - Enable synchronized output
    let data = b"\x1b[?2026h";
    let events = parser.parse(data);
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], OscEvent::SynchronizedOutputEnabled));
}

#[test]
fn test_synchronized_output_disable() {
    let mut parser = TerminalParser::new();
    // ESC [ ? 2026 l - Disable synchronized output
    let data = b"\x1b[?2026l";
    let events = parser.parse(data);
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], OscEvent::SynchronizedOutputDisabled));
}

#[test]
fn test_synchronized_output_full_cycle() {
    let mut parser = TerminalParser::new();
    // Enable then disable
    let enable_events = parser.parse(b"\x1b[?2026h");
    assert_eq!(enable_events.len(), 1);
    assert!(matches!(
        enable_events[0],
        OscEvent::SynchronizedOutputEnabled
    ));

    let disable_events = parser.parse(b"\x1b[?2026l");
    assert_eq!(disable_events.len(), 1);
    assert!(matches!(
        disable_events[0],
        OscEvent::SynchronizedOutputDisabled
    ));
}

#[test]
fn test_synchronized_output_with_alternate_screen() {
    let mut parser = TerminalParser::new();
    // Both modes in same sequence: CSI ? 2026 ; 1049 h
    let data = b"\x1b[?2026;1049h";
    let events = parser.parse(data);
    assert_eq!(events.len(), 2);
    assert!(matches!(events[0], OscEvent::SynchronizedOutputEnabled));
    assert!(matches!(events[1], OscEvent::AlternateScreenEnabled));
}

#[test]
fn test_synchronized_output_no_deduplication() {
    let mut parser = TerminalParser::new();
    // Unlike alternate screen, sync output does not deduplicate
    // Apps may toggle it multiple times
    let events1 = parser.parse(b"\x1b[?2026h");
    assert_eq!(events1.len(), 1);

    let events2 = parser.parse(b"\x1b[?2026h");
    assert_eq!(events2.len(), 1); // Should still emit
}

#[test]
fn test_synchronized_output_mixed_with_content() {
    let mut parser = TerminalParser::new();
    // Content mixed with sync output sequences
    let data = b"Hello\x1b[?2026hWorld\x1b[?2026l";
    let events = parser.parse(data);
    assert_eq!(events.len(), 2);
    assert!(matches!(events[0], OscEvent::SynchronizedOutputEnabled));
    assert!(matches!(events[1], OscEvent::SynchronizedOutputDisabled));
}

// ===========================================
// OSC 1337 - Virtual Environment tests
// ===========================================

#[test]
fn test_osc_1337_virtual_env() {
    let mut parser = TerminalParser::new();
    // OSC 1337 ; VirtualEnv=myenv ST (using ESC \ as terminator)
    let data = b"\x1b]1337;VirtualEnv=myenv\x1b\\";
    let events = parser.parse(data);
    assert_eq!(events.len(), 1);
    if let OscEvent::VirtualEnvChanged { name } = &events[0] {
        assert_eq!(name.as_deref(), Some("myenv"));
    } else {
        panic!("Expected VirtualEnvChanged, got {:?}", events[0]);
    }
}

#[test]
fn test_osc_1337_virtual_env_bel() {
    let mut parser = TerminalParser::new();
    // OSC 1337 ; VirtualEnv=myenv BEL (using BEL as terminator)
    let data = b"\x1b]1337;VirtualEnv=myenv\x07";
    let events = parser.parse(data);
    assert_eq!(events.len(), 1);
    if let OscEvent::VirtualEnvChanged { name } = &events[0] {
        assert_eq!(name.as_deref(), Some("myenv"));
    } else {
        panic!("Expected VirtualEnvChanged, got {:?}", events[0]);
    }
}

#[test]
fn test_osc_1337_virtual_env_clear() {
    let mut parser = TerminalParser::new();
    // First activate a venv
    parser.parse(b"\x1b]1337;VirtualEnv=myenv\x1b\\");
    // Then clear it
    let events = parser.parse(b"\x1b]1337;VirtualEnv=\x1b\\");
    assert_eq!(events.len(), 1);
    if let OscEvent::VirtualEnvChanged { name } = &events[0] {
        assert!(name.is_none());
    } else {
        panic!("Expected VirtualEnvChanged, got {:?}", events[0]);
    }
}

#[test]
fn test_osc_1337_virtual_env_deduplication() {
    let mut parser = TerminalParser::new();
    // First activation
    let events1 = parser.parse(b"\x1b]1337;VirtualEnv=myenv\x1b\\");
    assert_eq!(events1.len(), 1);

    // Duplicate - should be ignored
    let events2 = parser.parse(b"\x1b]1337;VirtualEnv=myenv\x1b\\");
    assert_eq!(events2.len(), 0);
}

// ===========================================
// Region filtering tests (parse_filtered)
// ===========================================

#[test]
fn test_parse_filtered_output_only() {
    let mut parser = TerminalParser::new();
    // Just regular output text, no OSC sequences - should pass through
    let result = parser.parse_filtered(b"Hello, World!\n");
    assert_eq!(result.events.len(), 0);
    assert_eq!(result.output, b"Hello, World!\n");
}

#[test]
fn test_parse_filtered_suppresses_prompt() {
    let mut parser = TerminalParser::new();
    // PromptStart -> prompt text -> PromptEnd
    // The prompt text should be suppressed
    let result = parser.parse_filtered(b"\x1b]133;A\x07user@host:~$ \x1b]133;B\x07");
    assert_eq!(result.events.len(), 2);
    assert!(matches!(result.events[0], OscEvent::PromptStart));
    assert!(matches!(result.events[1], OscEvent::PromptEnd));
    // Prompt text "user@host:~$ " should be suppressed
    assert_eq!(result.output, b"");
}

#[test]
fn test_parse_filtered_suppresses_user_input() {
    let mut parser = TerminalParser::new();
    // After PromptEnd (B), user types - this is the Input region
    // First set up the state: PromptStart -> PromptEnd
    parser.parse_filtered(b"\x1b]133;A\x07\x1b]133;B\x07");

    // Now user types "ls -la" and presses enter (CommandStart)
    let result = parser.parse_filtered(b"ls -la\x1b]133;C;ls -la\x07");
    assert_eq!(result.events.len(), 1);
    if let OscEvent::CommandStart { command } = &result.events[0] {
        assert_eq!(command.as_deref(), Some("ls -la"));
    } else {
        panic!("Expected CommandStart");
    }
    // User input "ls -la" should be suppressed (between B and C)
    assert_eq!(result.output, b"");
}

#[test]
fn test_parse_filtered_shows_command_output() {
    let mut parser = TerminalParser::new();
    // Set up state: we're in Output region after CommandStart
    parser.parse_filtered(b"\x1b]133;C;ls\x07");

    // Command output should be visible
    let result = parser.parse_filtered(b"file1.txt\nfile2.txt\n");
    assert_eq!(result.events.len(), 0);
    assert_eq!(result.output, b"file1.txt\nfile2.txt\n");
}

#[test]
fn test_parse_filtered_full_lifecycle() {
    let mut parser = TerminalParser::new();

    // Full command lifecycle:
    // 1. Prompt (suppressed)
    let r1 = parser.parse_filtered(b"\x1b]133;A\x07user@host:~$ \x1b]133;B\x07");
    assert_eq!(r1.output, b""); // Prompt suppressed

    // 2. User input (suppressed)
    let r2 = parser.parse_filtered(b"echo hello\x1b]133;C;echo hello\x07");
    assert_eq!(r2.output, b""); // Input suppressed

    // 3. Command output (visible)
    let r3 = parser.parse_filtered(b"hello\n");
    assert_eq!(r3.output, b"hello\n"); // Output visible

    // 4. Command ends
    let r4 = parser.parse_filtered(b"\x1b]133;D;0\x07");
    assert_eq!(r4.events.len(), 1);
    assert!(matches!(
        r4.events[0],
        OscEvent::CommandEnd { exit_code: 0 }
    ));

    // 5. Post-command shell artifacts (suppressed)
    let r5 = parser.parse_filtered(b"\x1b[?2004h%\r \r");
    assert_eq!(r5.output, b""); // Between D and A: suppressed

    // 6. Next prompt (suppressed)
    let r6 = parser.parse_filtered(b"\x1b]133;A\x07user@host:~$ \x1b]133;B\x07");
    assert_eq!(r6.output, b""); // Prompt suppressed
}

#[test]
fn test_parse_filtered_post_command_suppressed() {
    let mut parser = TerminalParser::new();

    // Set up: command start → output → command end
    parser.parse_filtered(b"\x1b]133;C;ls\x07");
    let r1 = parser.parse_filtered(b"file1\nfile2\n");
    assert_eq!(r1.output, b"file1\nfile2\n");

    parser.parse_filtered(b"\x1b]133;D;0\x07");

    // After command end, bytes should be suppressed (shell housekeeping)
    let r2 = parser.parse_filtered(b"\x1b[?2004h%\r \r\x1b[1m\x1b[7m%\x1b[27m\x1b[0m");
    assert_eq!(r2.output, b"");

    // Until the next prompt starts a new cycle
    parser.parse_filtered(b"\x1b]133;A\x07");
    parser.parse_filtered(b"\x1b]133;B\x07");
    parser.parse_filtered(b"\x1b]133;C;echo hi\x07");

    let r3 = parser.parse_filtered(b"hi\n");
    assert_eq!(r3.output, b"hi\n");
}

#[test]
fn test_parse_filtered_region_state_tracking() {
    let mut parser = TerminalParser::new();

    // Verify the region transitions are correct
    // Start in Output (default)
    assert_eq!(parser.performer.current_region, TerminalRegion::Output);

    parser.parse_filtered(b"\x1b]133;A\x07");
    assert_eq!(parser.performer.current_region, TerminalRegion::Prompt);

    parser.parse_filtered(b"\x1b]133;B\x07");
    assert_eq!(parser.performer.current_region, TerminalRegion::Input);

    parser.parse_filtered(b"\x1b]133;C\x07");
    assert_eq!(parser.performer.current_region, TerminalRegion::Output);

    parser.parse_filtered(b"\x1b]133;D;0\x07");
    // After CommandEnd, region switches to Prompt so post-command
    // artifacts (PROMPT_SP, bracketed paste, etc.) are suppressed.
    assert_eq!(parser.performer.current_region, TerminalRegion::Prompt);
}

#[test]
fn test_parse_filtered_handles_control_chars_in_output() {
    let mut parser = TerminalParser::new();
    // Ensure we're in Output region
    parser.parse_filtered(b"\x1b]133;C\x07");

    // Test that common control characters pass through
    let result = parser.parse_filtered(b"line1\r\nline2\tcolumn\n");
    assert_eq!(result.output, b"line1\r\nline2\tcolumn\n");
}

#[test]
fn test_parse_filtered_suppresses_control_chars_in_prompt() {
    let mut parser = TerminalParser::new();
    // Enter Prompt region
    parser.parse_filtered(b"\x1b]133;A\x07");

    // Control characters in prompt should be suppressed too
    let result = parser.parse_filtered(b"prompt\r\n");
    assert_eq!(result.output, b"");
}

// ===========================================
// SGR (color) passthrough tests
// ===========================================

#[test]
fn test_parse_filtered_passes_through_sgr_colors() {
    let mut parser = TerminalParser::new();
    // Ensure we're in Output region
    parser.parse_filtered(b"\x1b]133;C\x07");

    // SGR color sequence should be passed through: ESC[32m (green)
    let result = parser.parse_filtered(b"\x1b[32mgreen text\x1b[0m");
    assert_eq!(result.output, b"\x1b[32mgreen text\x1b[0m");
}

#[test]
fn test_parse_filtered_passes_through_multiple_sgr_params() {
    let mut parser = TerminalParser::new();
    // Ensure we're in Output region
    parser.parse_filtered(b"\x1b]133;C\x07");

    // SGR with multiple params: ESC[1;31m (bold red)
    let result = parser.parse_filtered(b"\x1b[1;31mbold red\x1b[0m");
    assert_eq!(result.output, b"\x1b[1;31mbold red\x1b[0m");
}

#[test]
fn test_parse_filtered_passes_through_256_color() {
    let mut parser = TerminalParser::new();
    // Ensure we're in Output region
    parser.parse_filtered(b"\x1b]133;C\x07");

    // 256-color mode: ESC[38;5;82m (foreground color 82)
    let result = parser.parse_filtered(b"\x1b[38;5;82mcolored\x1b[0m");
    assert_eq!(result.output, b"\x1b[38;5;82mcolored\x1b[0m");
}

#[test]
fn test_parse_filtered_passes_through_truecolor() {
    let mut parser = TerminalParser::new();
    // Ensure we're in Output region
    parser.parse_filtered(b"\x1b]133;C\x07");

    // Truecolor RGB: ESC[38;2;255;128;0m (orange foreground)
    let result = parser.parse_filtered(b"\x1b[38;2;255;128;0morange\x1b[0m");
    assert_eq!(result.output, b"\x1b[38;2;255;128;0morange\x1b[0m");
}

#[test]
fn test_parse_filtered_sgr_suppressed_in_prompt() {
    let mut parser = TerminalParser::new();
    // Enter Prompt region
    parser.parse_filtered(b"\x1b]133;A\x07");

    // SGR in prompt region should be suppressed
    let result = parser.parse_filtered(b"\x1b[32mprompt\x1b[0m");
    assert_eq!(result.output, b"");
}

#[test]
fn test_parse_filtered_sgr_suppressed_in_input() {
    let mut parser = TerminalParser::new();
    // Enter Input region (after prompt)
    parser.parse_filtered(b"\x1b]133;A\x07\x1b]133;B\x07");

    // SGR in input region should be suppressed
    let result = parser.parse_filtered(b"\x1b[32muser input\x1b[0m");
    assert_eq!(result.output, b"");
}

#[test]
fn test_parse_filtered_sgr_reset_only() {
    let mut parser = TerminalParser::new();
    // Ensure we're in Output region
    parser.parse_filtered(b"\x1b]133;C\x07");

    // Just reset sequence: ESC[0m
    let result = parser.parse_filtered(b"\x1b[0m");
    assert_eq!(result.output, b"\x1b[0m");

    // ESC[m (no params) is normalized to ESC[0m - this is semantically equivalent
    let result2 = parser.parse_filtered(b"\x1b[m");
    assert_eq!(result2.output, b"\x1b[0m");
}

#[test]
fn test_parse_filtered_sgr_complex_styling() {
    let mut parser = TerminalParser::new();
    // Ensure we're in Output region
    parser.parse_filtered(b"\x1b]133;C\x07");

    // Complex styling: bold, underline, italic, color
    // ESC[1;3;4;38;5;196m (bold, italic, underline, red 256-color)
    let result = parser.parse_filtered(b"\x1b[1;3;4;38;5;196mfancy\x1b[0m");
    assert_eq!(result.output, b"\x1b[1;3;4;38;5;196mfancy\x1b[0m");
}

// ===========================================
// CSI cursor movement & erase passthrough tests
// ===========================================

#[test]
fn test_parse_filtered_passes_through_cursor_up() {
    let mut parser = TerminalParser::new();
    // Ensure we're in Output region
    parser.parse_filtered(b"\x1b]133;C\x07");

    // ESC[3A - cursor up 3 rows
    let result = parser.parse_filtered(b"\x1b[3A");
    assert_eq!(result.output, b"\x1b[3A");
}

#[test]
fn test_parse_filtered_passes_through_erase_line() {
    let mut parser = TerminalParser::new();
    parser.parse_filtered(b"\x1b]133;C\x07");

    // ESC[2K - erase entire line
    let result = parser.parse_filtered(b"\x1b[2K");
    assert_eq!(result.output, b"\x1b[2K");
}

#[test]
fn test_parse_filtered_passes_through_erase_screen() {
    let mut parser = TerminalParser::new();
    parser.parse_filtered(b"\x1b]133;C\x07");

    // ESC[2J - erase entire screen
    let result = parser.parse_filtered(b"\x1b[2J");
    assert_eq!(result.output, b"\x1b[2J");
}

#[test]
fn test_parse_filtered_passes_through_mouse_mode() {
    let mut parser = TerminalParser::new();
    parser.parse_filtered(b"\x1b]133;C\x07");

    // ESC[?1000h - enable mouse reporting in Output region
    // Should appear in visible output bytes AND emit an OscEvent
    let result = parser.parse_filtered(b"\x1b[?1000h");
    assert_eq!(result.output, b"\x1b[?1000h");
    assert_eq!(result.events.len(), 1);
    assert!(matches!(result.events[0], OscEvent::MouseReportingEnabled));
}

#[test]
fn test_parse_filtered_suppresses_csi_in_prompt() {
    let mut parser = TerminalParser::new();
    // Enter Prompt region
    parser.parse_filtered(b"\x1b]133;A\x07");

    // ESC[H in Prompt region — should be suppressed from output
    let result = parser.parse_filtered(b"\x1b[H");
    assert_eq!(result.output, b"");
}

// ===========================================
// ESC dispatch passthrough tests
// ===========================================

#[test]
fn test_parse_filtered_passes_through_esc_equals() {
    let mut parser = TerminalParser::new();
    parser.parse_filtered(b"\x1b]133;C\x07");

    // ESC= (DECKPAM - application keypad mode, sent by vim)
    let result = parser.parse_filtered(b"\x1b=");
    assert_eq!(result.output, b"\x1b=");
}

#[test]
fn test_parse_filtered_passes_through_esc_with_intermediate() {
    let mut parser = TerminalParser::new();
    parser.parse_filtered(b"\x1b]133;C\x07");

    // ESC(B - DEC designate G0 character set (US ASCII)
    let result = parser.parse_filtered(b"\x1b(B");
    assert_eq!(result.output, b"\x1b(B");
}

#[test]
fn test_parse_filtered_suppresses_esc_in_prompt() {
    let mut parser = TerminalParser::new();
    // Enter Prompt region
    parser.parse_filtered(b"\x1b]133;A\x07");

    // ESC= in Prompt region — should be suppressed
    let result = parser.parse_filtered(b"\x1b=");
    assert_eq!(result.output, b"");
}

// ===========================================
// New OscEvent variants — mouse & bracketed paste
// ===========================================

#[test]
fn test_mouse_reporting_enable() {
    let mut parser = TerminalParser::new();
    let events = parser.parse(b"\x1b[?1000h");
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], OscEvent::MouseReportingEnabled));
}

#[test]
fn test_mouse_reporting_disable() {
    let mut parser = TerminalParser::new();
    let events = parser.parse(b"\x1b[?1000l");
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], OscEvent::MouseReportingDisabled));
}

#[test]
fn test_sgr_mouse_enable() {
    let mut parser = TerminalParser::new();
    let events = parser.parse(b"\x1b[?1006h");
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], OscEvent::SgrMouseEnabled));
}

#[test]
fn test_sgr_mouse_disable() {
    let mut parser = TerminalParser::new();
    let events = parser.parse(b"\x1b[?1006l");
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], OscEvent::SgrMouseDisabled));
}

#[test]
fn test_bracketed_paste_enable() {
    let mut parser = TerminalParser::new();
    let events = parser.parse(b"\x1b[?2004h");
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], OscEvent::BracketedPasteEnabled));
}

#[test]
fn test_bracketed_paste_disable() {
    let mut parser = TerminalParser::new();
    let events = parser.parse(b"\x1b[?2004l");
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], OscEvent::BracketedPasteDisabled));
}

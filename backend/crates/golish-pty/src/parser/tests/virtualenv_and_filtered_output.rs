use super::*;

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
    // Prompt text "user@host:~$ " should be suppressed from `output`
    // (timeline renderer) ...
    assert_eq!(result.output, b"");
    // ... but the same text MUST be present in `prompt_visible` so
    // the Warp-style stdin_wait detector can read PS1/PS2/PS3 prompts.
    assert_eq!(result.prompt_visible, b"user@host:~$ ");
}

#[test]
fn test_parse_filtered_prompt_visible_captures_zsh_select_ps2() {
    // Regression for the bug where zsh's `select> ` PS2 continuation
    // prompt was invisible to the stdin_wait detector. After OSC 133
    // shell integration calls PromptEnd at the start of each readline
    // iteration the region is `Input`, which used to filter the prompt
    // bytes out of every downstream signal. `prompt_visible` now
    // preserves them so the detector can match `>` line endings.
    let mut parser = TerminalParser::new();
    // Set up state to match a real zsh `select` continuation: a B
    // (PromptEnd) lands first, then the prompt text streams in.
    parser.parse_filtered(b"\x1b]133;B\x07");
    let result = parser.parse_filtered(b"select> ");
    // The timeline renderer still sees nothing — it's still
    // technically in the Input region for OSC 133 purposes.
    assert_eq!(result.output, b"");
    // But the detector buffer sees the full prompt text.
    assert_eq!(result.prompt_visible, b"select> ");
}

#[test]
fn test_parse_filtered_prompt_visible_captures_bash_ps3() {
    // Same idea for bash's PS3 prompt (`#? `) which is emitted while
    // the `select` builtin runs. The bytes land in the Output region
    // so they were already visible in `output`, but we should still
    // mirror them into `prompt_visible` so the detector pipeline can
    // treat both regions uniformly.
    let mut parser = TerminalParser::new();
    parser.parse_filtered(b"\x1b]133;C;select yn in \"Yes\" \"No\"\x07");
    let result = parser.parse_filtered(b"1) Yes\n2) No\n#? ");
    assert_eq!(result.output, b"1) Yes\n2) No\n#? ");
    assert_eq!(result.prompt_visible, b"1) Yes\n2) No\n#? ");
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

#[test]
fn test_parse_filtered_consumes_dsr_cursor_query() {
    let mut parser = TerminalParser::new();
    // Output region.
    parser.parse_filtered(b"\x1b]133;C\x07");

    // The CSI 6 n cursor-position query must be answered out-of-band
    // (written back to the PTY), never echoed into the rendered output
    // stream — even when it lands between visible bytes.
    let result = parser.parse_filtered(b"before\x1b[6nafter");
    assert_eq!(result.output, b"beforeafter");
    assert_eq!(result.events.len(), 1);
    assert!(matches!(result.events[0], OscEvent::CursorPositionRequest));
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

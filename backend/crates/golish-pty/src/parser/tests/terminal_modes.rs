use super::*;

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

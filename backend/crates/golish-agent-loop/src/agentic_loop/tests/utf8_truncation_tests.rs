#[test]
fn test_utf8_safe_truncation_ascii() {
    let text = "Hello, World!";
    let mut end = 5;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    assert_eq!(&text[..end], "Hello");
}

#[test]
fn test_utf8_safe_truncation_multibyte() {
    // "─" is 3 bytes (E2 94 80), testing truncation at various positions
    let text = "abc─def"; // a=0, b=1, c=2, ─=3-5, d=6, e=7, f=8

    // Truncate at position 4 (middle of ─)
    let mut end = 4;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    assert_eq!(end, 3); // Should back up to position 3 (start of ─)
    assert_eq!(&text[..end], "abc");

    // Truncate at position 5 (still in ─)
    let mut end = 5;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    assert_eq!(end, 3);
    assert_eq!(&text[..end], "abc");

    // Truncate at position 6 (after ─)
    let mut end = 6;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    assert_eq!(end, 6);
    assert_eq!(&text[..end], "abc─");
}

#[test]
fn test_utf8_safe_truncation_emoji() {
    // Emoji like 🎉 is 4 bytes
    let text = "Hi🎉!"; // H=0, i=1, 🎉=2-5, !=6

    // Truncate at position 3 (middle of emoji)
    let mut end = 3;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    assert_eq!(end, 2);
    assert_eq!(&text[..end], "Hi");

    // Truncate at position 6 (after emoji)
    let mut end = 6;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    assert_eq!(end, 6);
    assert_eq!(&text[..end], "Hi🎉");
}

#[test]
fn test_utf8_safe_truncation_mixed_box_drawing() {
    // Box drawing characters like those that caused the original panic
    let text = "Summary:\n─────────";
    let target = 12; // Might land in middle of a box char

    let mut end = target.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }

    // Should not panic and result should be valid UTF-8
    let truncated = &text[..end];
    assert!(truncated.len() <= target);
    // Verify it's valid UTF-8 by checking we can iterate chars
    assert!(truncated.chars().count() > 0);
}

//! UTF-8 boundary buffering for PTY output.
//!
//! When reading raw bytes from a PTY in fixed-size chunks, multi-byte
//! UTF-8 sequences can be split across reads. The helpers in this module
//! buffer the trailing 1–3 bytes of an incomplete sequence and prepend
//! them to the next read so the emitter never produces invalid UTF-8.

/// Messages sent from the PTY reader thread to the output emitter thread.
///
/// The reader thread sends raw output bytes through this channel so the
/// emitter thread can coalesce bursts of small reads into batched IPC
/// events.
pub(super) enum OutputMessage {
    Data(Vec<u8>),
    Eof,
}

/// Buffer for holding incomplete UTF-8 sequences between PTY reads.
/// Max UTF-8 char is 4 bytes, so we buffer up to 3 trailing bytes.
pub(super) struct Utf8IncompleteBuffer {
    bytes: [u8; 3],
    len: u8,
}

impl Utf8IncompleteBuffer {
    pub(super) fn new() -> Self {
        Self {
            bytes: [0; 3],
            len: 0,
        }
    }

    pub(super) fn has_pending(&self) -> bool {
        self.len > 0
    }

    pub(super) fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }

    fn clear(&mut self) {
        self.len = 0;
    }

    fn store(&mut self, bytes: &[u8]) {
        let len = bytes.len().min(3);
        self.bytes[..len].copy_from_slice(&bytes[..len]);
        self.len = len as u8;
    }
}

/// Find boundary where valid complete UTF-8 ends.
/// Returns the index up to which the data is valid UTF-8.
fn find_valid_utf8_boundary(data: &[u8]) -> usize {
    if data.is_empty() {
        return 0;
    }

    // Check last 1–3 bytes for incomplete sequences.
    for check_len in 1..=3.min(data.len()) {
        let start_idx = data.len() - check_len;
        if is_incomplete_utf8_start(&data[start_idx..]) {
            return start_idx;
        }
    }

    // Verify entire buffer.
    match std::str::from_utf8(data) {
        Ok(_) => data.len(),
        Err(e) => e.valid_up_to(),
    }
}

/// Check if bytes are start of an incomplete UTF-8 sequence.
fn is_incomplete_utf8_start(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }

    let expected_len = match bytes[0] {
        b if b & 0b1000_0000 == 0 => 1,           // ASCII
        b if b & 0b1110_0000 == 0b1100_0000 => 2, // 2-byte
        b if b & 0b1111_0000 == 0b1110_0000 => 3, // 3-byte
        b if b & 0b1111_1000 == 0b1111_0000 => 4, // 4-byte
        _ => return false,                        // Invalid lead or continuation byte
    };

    if bytes.len() >= expected_len {
        return false; // Complete sequence
    }

    // Verify remaining bytes are valid continuation bytes.
    bytes[1..].iter().all(|&b| b & 0b1100_0000 == 0b1000_0000)
}

/// Process bytes into valid UTF-8, buffering incomplete sequences.
pub(super) fn process_utf8_with_buffer(
    buf: &mut Utf8IncompleteBuffer,
    data: &[u8],
) -> String {
    if !buf.has_pending() {
        let valid_len = find_valid_utf8_boundary(data);
        if valid_len < data.len() {
            buf.store(&data[valid_len..]);
        }
        return String::from_utf8_lossy(&data[..valid_len]).to_string();
    }

    // Combine pending + new data.
    let mut combined = Vec::with_capacity(buf.as_slice().len() + data.len());
    combined.extend_from_slice(buf.as_slice());
    combined.extend_from_slice(data);
    buf.clear();

    let valid_len = find_valid_utf8_boundary(&combined);
    if valid_len < combined.len() {
        buf.store(&combined[valid_len..]);
    }
    String::from_utf8_lossy(&combined[..valid_len]).to_string()
}

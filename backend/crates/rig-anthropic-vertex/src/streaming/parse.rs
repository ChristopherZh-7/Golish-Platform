//! SSE line parsing.

use crate::error::AnthropicVertexError;
use crate::types::StreamEvent;

use super::StreamingResponse;

impl StreamingResponse {
    /// Parse an SSE line into a stream event.
    ///
    /// SSE format is:
    /// ```text
    /// event: content_block_delta
    /// data: {"type":"content_block_delta",...}
    /// ```
    ///
    /// We must only match `data: ` at the START of a line, not inside
    /// JSON content. This prevents false matches when streamed text
    /// contains `"data: "` strings.
    pub(super) fn parse_sse_line(line: &str) -> Option<Result<StreamEvent, AnthropicVertexError>> {
        let line = line.trim();

        if line.is_empty() || line.starts_with(':') {
            return None;
        }

        // Parse SSE properly: find a data line that starts at beginning
        // of a line. We take the LAST `data:` line in case there are
        // multiple (shouldn't happen, but defensive coding against
        // malformed responses).
        let mut data_content: Option<&str> = None;

        for subline in line.split('\n') {
            let subline = subline.trim();
            // Only match "data: " at the START of the line.
            if let Some(content) = subline.strip_prefix("data: ") {
                data_content = Some(content);
            }
        }

        let data_content = match data_content {
            Some(d) => d.trim(),
            None => {
                tracing::trace!(
                    "SSE: No data field found in: {}",
                    &line[..line.len().min(100)]
                );
                return None;
            }
        };

        // Skip [DONE] marker.
        if data_content == "[DONE]" {
            tracing::debug!("SSE: Received [DONE] marker");
            return None;
        }

        match serde_json::from_str::<StreamEvent>(data_content) {
            Ok(ref event) => Some(Ok(event.clone())),
            Err(e) => {
                tracing::warn!(
                    "SSE: Failed to parse event: {} - data: {}",
                    e,
                    &data_content[..data_content.len().min(200)]
                );
                Some(Err(AnthropicVertexError::ParseError(format!(
                    "Failed to parse stream event: {} - data: {}",
                    e, data_content
                ))))
            }
        }
    }
}

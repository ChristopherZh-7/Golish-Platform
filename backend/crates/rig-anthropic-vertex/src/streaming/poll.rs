//! `Stream` trait impl: byte-stream pump + buffer-based SSE-line splitting.

use std::pin::Pin;
use std::task::{Context, Poll};

use futures::Stream;

use crate::error::AnthropicVertexError;

use super::{StreamChunk, StreamingResponse};

impl Stream for StreamingResponse {
    type Item = Result<StreamChunk, AnthropicVertexError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.done {
            tracing::trace!("poll_next: already done");
            return Poll::Ready(None);
        }

        loop {
            // Check if we have complete lines in the buffer.
            if let Some(newline_pos) = self.buffer.find("\n\n") {
                let line = self.buffer[..newline_pos].to_string();
                self.buffer = self.buffer[newline_pos + 2..].to_string();
                tracing::trace!(
                    "poll_next: found SSE line, {} chars remaining in buffer",
                    self.buffer.len()
                );

                if let Some(result) = Self::parse_sse_line(&line) {
                    match result {
                        Ok(event) => {
                            let chunk = self.event_to_chunk(event);
                            if let Some(chunk) = chunk {
                                return Poll::Ready(Some(Ok(chunk)));
                            }
                            // Continue processing if we got a non-yielding event.
                            continue;
                        }
                        Err(e) => return Poll::Ready(Some(Err(e))),
                    }
                }
                continue;
            }

            // Need more data from the stream.
            match Pin::new(&mut self.inner).poll_next(cx) {
                Poll::Ready(Some(Ok(bytes))) => {
                    let bytes_len = bytes.len();
                    if let Ok(text) = std::str::from_utf8(&bytes) {
                        self.buffer.push_str(text);
                        tracing::debug!(
                            "poll_next: received {} bytes, buffer now {} chars",
                            bytes_len,
                            self.buffer.len()
                        );
                        if self.buffer.len() < 500 {
                            tracing::debug!("poll_next: buffer content: {:?}", self.buffer);
                        }
                    } else {
                        tracing::warn!(
                            "poll_next: received {} bytes but not valid UTF-8",
                            bytes_len
                        );
                    }
                    // Continue to process the buffer.
                }
                Poll::Ready(Some(Err(e))) => {
                    tracing::error!("poll_next: stream error: {}", e);
                    return Poll::Ready(Some(Err(AnthropicVertexError::StreamError(
                        e.to_string(),
                    ))));
                }
                Poll::Ready(None) => {
                    tracing::info!(
                        "poll_next: stream ended, buffer has {} chars remaining",
                        self.buffer.len()
                    );
                    if !self.buffer.is_empty() {
                        tracing::debug!(
                            "poll_next: remaining buffer: {:?}",
                            &self.buffer[..self.buffer.len().min(500)]
                        );
                    }
                    self.done = true;
                    // Process any remaining buffer.
                    if !self.buffer.is_empty() {
                        if let Some(result) = Self::parse_sse_line(&self.buffer) {
                            self.buffer.clear();
                            match result {
                                Ok(event) => {
                                    if let Some(chunk) = self.event_to_chunk(event) {
                                        return Poll::Ready(Some(Ok(chunk)));
                                    }
                                }
                                Err(e) => return Poll::Ready(Some(Err(e))),
                            }
                        }
                    }
                    return Poll::Ready(None);
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

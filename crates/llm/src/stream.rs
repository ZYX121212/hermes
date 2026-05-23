// crates/llm/src/stream.rs
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures::Stream;

/// SSE chunk stream that parses both Anthropic and OpenAI streaming formats.
pub struct SseChunkStream {
    byte_stream: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>,
    buffer: String,
}

impl SseChunkStream {
    pub fn new(byte_stream: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>) -> Self {
        Self {
            byte_stream,
            buffer: String::new(),
        }
    }
}

impl Stream for SseChunkStream {
    type Item = anyhow::Result<String>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            // Process lines buffered so far
            if let Some(pos) = self.buffer.find('\n') {
                let line = self.buffer[..pos].to_string();
                self.buffer = self.buffer[pos + 1..].to_string();

                let line = line.trim().to_string();
                if line.is_empty() || line.starts_with(':') {
                    continue;
                }

                if line == "data: [DONE]" {
                    return Poll::Ready(None);
                }

                if let Some(data) = line.strip_prefix("data: ") {
                    match serde_json::from_str::<serde_json::Value>(data) {
                        Ok(val) => {
                            // Check for stream stop signals
                            if val["type"].as_str() == Some("message_stop") {
                                return Poll::Ready(None);
                            }
                            if val["choices"][0]["finish_reason"].as_str() == Some("stop") {
                                return Poll::Ready(None);
                            }
                            // Anthropic format: content_block_delta
                            if let Some(text) = val["delta"]["text"].as_str() {
                                if !text.is_empty() {
                                    return Poll::Ready(Some(Ok(text.to_string())));
                                }
                            }
                            // OpenAI format: choices[0].delta.content
                            if let Some(text) = val["choices"][0]["delta"]["content"].as_str() {
                                if !text.is_empty() {
                                    return Poll::Ready(Some(Ok(text.to_string())));
                                }
                            }
                        }
                        Err(e) => {
                            tracing::debug!(error = %e, "SSE data line parse failed, skipping chunk");
                            continue;
                        }
                    }
                }
                continue;
            }

            // Need more bytes
            match self.byte_stream.as_mut().poll_next(cx) {
                Poll::Ready(Some(Ok(chunk))) => {
                    match String::from_utf8(chunk.to_vec()) {
                        Ok(s) => self.buffer.push_str(&s),
                        Err(_) => {
                            tracing::debug!("Non-UTF8 bytes in stream, using lossy conversion");
                            self.buffer.push_str(&String::from_utf8_lossy(&chunk));
                        }
                    }
                }
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Some(Err(anyhow::anyhow!("Stream error: {e}"))));
                }
                Poll::Ready(None) => {
                    return Poll::Ready(None);
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

impl Unpin for SseChunkStream {}

// crates/llm/src/retry.rs
// Exponential backoff retry helper for LLM API calls.

use std::future::Future;
use std::time::Duration;

/// Retry an async operation with exponential backoff.
///
/// Retries only on transient failures: 5xx server errors, 429 rate limits,
/// network errors, and timeouts. 4xx client errors are returned immediately.
///
/// Backoff: 1s → 2s → 4s → 8s (up to `max_retries` attempts after the initial try).
pub async fn with_retry<F, Fut, T>(
    operation_name: &str,
    max_retries: u32,
    mut f: F,
) -> Result<T, anyhow::Error>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, anyhow::Error>>,
{
    let mut attempt: u32 = 0;
    loop {
        match f().await {
            Ok(val) => return Ok(val),
            Err(e) => {
                if attempt >= max_retries {
                    return Err(e);
                }
                let err_str = e.to_string().to_lowercase();
                let is_retryable = err_str.contains("500")
                    || err_str.contains("502")
                    || err_str.contains("503")
                    || err_str.contains("504")
                    || err_str.contains("429")
                    || err_str.contains("timeout")
                    || err_str.contains("timed out")
                    || err_str.contains("connection")
                    || err_str.contains("tls")
                    || err_str.contains("dns")
                    || err_str.contains("eof");

                if !is_retryable {
                    return Err(e);
                }

                attempt += 1;
                let delay = Duration::from_secs(1u64 << (attempt - 1)); // 1, 2, 4, 8s
                tracing::warn!(
                    operation = operation_name,
                    attempt,
                    max_retries,
                    delay_ms = delay.as_millis(),
                    "LLM request failed, retrying"
                );
                tokio::time::sleep(delay).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn test_no_retry_on_success() {
        let calls = Arc::new(AtomicU32::new(0));
        let c = calls.clone();
        let result = with_retry("test", 3, move || {
            c.fetch_add(1, Ordering::SeqCst);
            let r: Result<&str, anyhow::Error> = Ok("ok");
            async { r }
        })
        .await;
        assert!(result.is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_retry_on_503() {
        let calls = Arc::new(AtomicU32::new(0));
        let c = calls.clone();
        let result = with_retry("test", 3, move || {
            let n = c.fetch_add(1, Ordering::SeqCst);
            async move {
                if n < 2 {
                    Err(anyhow::anyhow!("HTTP 503 Service Unavailable"))
                } else {
                    Ok("recovered")
                }
            }
        })
        .await;
        assert!(result.is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 3); // 2 failures + 1 success
    }

    #[tokio::test]
    async fn test_no_retry_on_400() {
        let calls = Arc::new(AtomicU32::new(0));
        let c = calls.clone();
        let result: Result<&str, _> = with_retry("test", 3, move || {
            c.fetch_add(1, Ordering::SeqCst);
            async { Err(anyhow::anyhow!("HTTP 400 Bad Request")) }
        })
        .await;
        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_exhaustion() {
        let calls = Arc::new(AtomicU32::new(0));
        let c = calls.clone();
        let result: Result<&str, _> = with_retry("test", 2, move || {
            c.fetch_add(1, Ordering::SeqCst);
            async { Err(anyhow::anyhow!("connection timeout")) }
        })
        .await;
        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 3); // 1 initial + 2 retries
    }
}

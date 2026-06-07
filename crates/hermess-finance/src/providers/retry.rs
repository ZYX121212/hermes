// Shared retry logic with exponential backoff for HTTP providers.
use std::future::Future;
use std::time::Duration;

/// Retry configuration.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_retries: u32,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
}

impl RetryConfig {
    pub fn with_delays(max_retries: u32, base_delay_ms: u64, max_delay_ms: u64) -> Self {
        Self {
            max_retries,
            base_delay_ms,
            max_delay_ms,
        }
    }
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay_ms: 500,
            max_delay_ms: 10_000,
        }
    }
}

/// Execute a fallible async operation with exponential backoff retry.
/// Retries on any error; the caller should only wrap network-level operations.
pub async fn with_retry<F, Fut, T, E>(config: &RetryConfig, label: &str, mut f: F) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let mut attempt = 0;
    loop {
        match f().await {
            Ok(val) => return Ok(val),
            Err(e) => {
                attempt += 1;
                if attempt > config.max_retries {
                    tracing::warn!(%label, attempt, error = %e, "All retries exhausted");
                    return Err(e);
                }
                let delay = (config.base_delay_ms * 2u64.pow(attempt - 1)).min(config.max_delay_ms);
                tracing::debug!(
                    %label,
                    attempt,
                    delay_ms = delay,
                    error = %e,
                    "Retrying after failure"
                );
                tokio::time::sleep(Duration::from_millis(delay)).await;
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
    async fn succeeds_first_try() {
        let cfg = RetryConfig::default();
        let result = with_retry(&cfg, "test", || async { Ok::<_, &str>(42) }).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn retries_then_succeeds() {
        let cfg = RetryConfig {
            max_retries: 3,
            base_delay_ms: 1,
            max_delay_ms: 10,
        };
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let result = with_retry(&cfg, "test", move || {
            let c = c.clone();
            async move {
                let n = c.fetch_add(1, Ordering::SeqCst);
                if n < 2 {
                    Err("fail")
                } else {
                    Ok(99)
                }
            }
        })
        .await;
        assert_eq!(result.unwrap(), 99);
        assert_eq!(counter.load(Ordering::SeqCst), 3); // 2 fails + 1 success
    }

    #[tokio::test]
    async fn exhausts_retries() {
        let cfg = RetryConfig {
            max_retries: 2,
            base_delay_ms: 1,
            max_delay_ms: 10,
        };
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let result = with_retry(&cfg, "test", move || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err::<(), _>("always fail")
            }
        })
        .await;
        assert!(result.is_err());
        assert_eq!(counter.load(Ordering::SeqCst), 3); // initial + 2 retries
    }
}

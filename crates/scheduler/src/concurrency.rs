// crates/scheduler/src/concurrency.rs
use std::sync::Arc;
use tokio::sync::Semaphore;

/// Concurrency controller: limits the maximum number of simultaneously
/// executing tool calls via a tokio Semaphore.
pub struct ConcurrencyLimit {
    semaphore: Arc<Semaphore>,
}

impl ConcurrencyLimit {
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrent.max(1))),
        }
    }

    /// Acquire a permit before executing a tool call.
    /// Returns a guard that releases the permit on drop.
    pub async fn acquire(&self) -> Result<tokio::sync::OwnedSemaphorePermit, anyhow::Error> {
        self.semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| anyhow::anyhow!("Concurrency semaphore closed — agent is shutting down"))
    }

    pub fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_acquire_and_release() {
        let limit = ConcurrencyLimit::new(2);
        assert_eq!(limit.available_permits(), 2);

        let p1 = limit.acquire().await.unwrap();
        assert_eq!(limit.available_permits(), 1);

        let p2 = limit.acquire().await.unwrap();
        assert_eq!(limit.available_permits(), 0);

        drop(p1);
        // Permit is returned asynchronously via semaphore
        tokio::task::yield_now().await;
        assert_eq!(limit.available_permits(), 1);

        drop(p2);
        tokio::task::yield_now().await;
        assert_eq!(limit.available_permits(), 2);
    }

    #[test]
    fn test_min_concurrency_is_one() {
        let limit = ConcurrencyLimit::new(0);
        assert_eq!(limit.available_permits(), 1);
    }
}

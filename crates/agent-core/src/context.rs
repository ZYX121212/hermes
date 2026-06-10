// crates/agent-core/src/context.rs
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Execution context for an agent run.
/// Holds the optional task and a cancellable stop signal (Ctrl-C).
pub struct Context {
    task: Option<String>,
    stop_flag: Arc<AtomicBool>,
    /// When > 0, the agent runs exactly this many iterations then stops.
    /// 0 means unlimited (runs until Ctrl+C).
    max_iterations: u64,
    iteration: Arc<AtomicU64>,
    /// In interactive mode, the task is updated each iteration from stdin.
    interactive: bool,
    /// Mutable task buffer for interactive mode.
    interactive_task: Arc<Mutex<String>>,
}

impl Context {
    pub fn new(task: Option<String>) -> Self {
        let has_task = task.is_some();
        let ctx = Self {
            task,
            stop_flag: Arc::new(AtomicBool::new(false)),
            max_iterations: if has_task { 1 } else { 0 },
            iteration: Arc::new(AtomicU64::new(0)),
            interactive: false,
            interactive_task: Arc::new(Mutex::new(String::new())),
        };
        let flag = Arc::clone(&ctx.stop_flag);
        tokio::spawn(async move {
            match tokio::signal::ctrl_c().await {
                Ok(()) => {
                    tracing::info!("received Ctrl-C, signalling stop");
                    flag.store(true, Ordering::Relaxed);
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to register Ctrl-C handler — agent will not respond to Ctrl-C");
                }
            }
        });
        ctx
    }

    /// Create an interactive context that runs until Ctrl+C,
    /// reading new tasks from stdin each iteration.
    pub fn interactive() -> Self {
        Self::interactive_with_task(None)
    }

    /// Create an interactive context, optionally seeding the first task.
    pub fn interactive_with_task(task: Option<String>) -> Self {
        let ctx = Self {
            task: None,
            stop_flag: Arc::new(AtomicBool::new(false)),
            max_iterations: 0, // unlimited
            iteration: Arc::new(AtomicU64::new(0)),
            interactive: true,
            interactive_task: Arc::new(Mutex::new(task.unwrap_or_default())),
        };
        let flag = Arc::clone(&ctx.stop_flag);
        tokio::spawn(async move {
            match tokio::signal::ctrl_c().await {
                Ok(()) => {
                    tracing::info!("received Ctrl-C, signalling stop");
                    flag.store(true, Ordering::Relaxed);
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to register Ctrl-C handler — agent will not respond to Ctrl-C");
                }
            }
        });
        ctx
    }

    pub fn task(&self) -> Option<&str> {
        if self.interactive {
            // Read new task from stdin (blocking, called from async context via spawn_blocking)
            None // Handled by the agent's observe() method
        } else {
            self.task.as_deref()
        }
    }

    /// Whether this is an interactive session.
    pub fn is_interactive(&self) -> bool {
        self.interactive
    }

    /// Read the next task from stdin and return it.
    /// This is a blocking call.
    pub fn next_interactive_task(&self) -> String {
        use std::io::Write;
        let mut input = String::new();
        print!("\n\x1b[1;36m▸\x1b[0m ");
        std::io::stdout().flush().ok();
        std::io::stdin().read_line(&mut input).ok();
        let trimmed = input.trim().to_string();
        if !trimmed.is_empty() {
            *self
                .interactive_task
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = trimmed.clone();
        }
        trimmed
    }

    /// Take a seeded interactive task, if one exists.
    pub fn take_seeded_interactive_task(&self) -> Option<String> {
        let mut task = self
            .interactive_task
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if task.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut *task))
        }
    }

    /// Replace the internal stop flag with an external one (for TUI integration).
    pub fn with_stop_flag(mut self, flag: Arc<AtomicBool>) -> Self {
        self.stop_flag = flag;
        self
    }

    /// Returns a clone of the internal stop flag for external observers (TUI, etc.)
    /// to request agent shutdown.
    pub fn stop_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.stop_flag)
    }

    /// Signal the agent to stop (used for interactive exit).
    pub fn signal_stop(&self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }

    /// Check if the agent should stop. Returns true if:
    /// - Ctrl+C was received, or
    /// - The max iteration count has been reached (non-interactive only).
    ///
    /// This is a pure predicate — it does NOT modify state. Callers
    /// must invoke `advance_iteration` once per completed iteration.
    pub fn should_stop(&self) -> bool {
        if self.stop_flag.load(Ordering::Relaxed) {
            return true;
        }
        if self.interactive {
            return false; // Only Ctrl-C stops interactive mode
        }
        if self.max_iterations > 0 {
            self.iteration.load(Ordering::Relaxed) > self.max_iterations
        } else {
            false
        }
    }

    /// Increment the iteration counter. Call once per completed iteration
    /// (after evolve + summarize). Returns the new count.
    pub fn advance_iteration(&self) -> u64 {
        self.iteration.fetch_add(1, Ordering::Relaxed) + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn interactive_with_task_seeds_first_task_once() {
        let ctx = Context::interactive_with_task(Some("first task".into()));

        assert!(ctx.is_interactive());
        assert_eq!(
            ctx.take_seeded_interactive_task().as_deref(),
            Some("first task")
        );
        assert!(ctx.take_seeded_interactive_task().is_none());
        assert!(!ctx.should_stop());
    }
}

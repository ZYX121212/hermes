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
            tokio::signal::ctrl_c().await.ok();
            tracing::info!("received Ctrl-C, signalling stop");
            flag.store(true, Ordering::Relaxed);
        });
        ctx
    }

    /// Create an interactive context that runs until Ctrl+C,
    /// reading new tasks from stdin each iteration.
    pub fn interactive() -> Self {
        let ctx = Self {
            task: None,
            stop_flag: Arc::new(AtomicBool::new(false)),
            max_iterations: 0, // unlimited
            iteration: Arc::new(AtomicU64::new(0)),
            interactive: true,
            interactive_task: Arc::new(Mutex::new(String::new())),
        };
        let flag = Arc::clone(&ctx.stop_flag);
        tokio::spawn(async move {
            tokio::signal::ctrl_c().await.ok();
            tracing::info!("received Ctrl-C, signalling stop");
            flag.store(true, Ordering::Relaxed);
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
            *self.interactive_task.lock().unwrap() = trimmed.clone();
        }
        trimmed
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
    pub fn should_stop(&self) -> bool {
        if self.stop_flag.load(Ordering::Relaxed) {
            return true;
        }
        if self.interactive {
            return false; // Only Ctrl-C stops interactive mode
        }
        if self.max_iterations > 0 {
            let count = self.iteration.fetch_add(1, Ordering::Relaxed) + 1;
            count > self.max_iterations
        } else {
            false
        }
    }
}

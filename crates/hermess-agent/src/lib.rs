//! Default agent implementation with a five-phase self-evolution loop.
//!
//! The `HermesAgent` orchestrates planning, scheduling, reflection, evolution,
//! and memory consolidation in a continuous reAct-style cycle.


pub mod agent;
pub mod crypto;
pub mod curator;
pub mod distiller;
pub mod input_guard;
pub mod mimo;
pub mod subagent;

pub use agent::SmallHermesAgent;
pub use crypto::DataVault;
pub use curator::{CuratorAction, CuratorReview, OutdatedRef, SkillCurator, SkillPatcher};
pub use distiller::{DistillResult, DistillTrigger, SkillDistiller};
pub use input_guard::{InjectionReport, PiiRedactor, PiiSummary, PromptInjectionDetector, RiskLevel};
pub use mimo::{AggregateStrategy, MiMoCandidate, MiMoMode, MiMoResult, MiMoRunner};
pub use subagent::{run_subagent, SubAgentResult, SubAgentRunnerImpl};

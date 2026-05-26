// crates/evolution/src/lib.rs
pub mod engine;
pub mod insight;
pub mod scorer;
pub mod weight;

pub use engine::EvolutionEngine;
pub use insight::InsightStats;
pub use scorer::Scorer;
pub use weight::{adaptive_lr, clamp, AtomicF64};

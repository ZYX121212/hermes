// crates/memory/src/lib.rs
pub mod dedup;
pub mod embedding;
pub mod preload;
pub mod vector;
pub mod working;

pub use dedup::{deduplicate_chunks, DedupConfig, DedupStats};
pub use embedding::{Embedder, HashEmbedder, VoyageEmbedder};
pub use preload::{preload_knowledge_base, KnowledgeBaseStats};
pub use vector::{MockMemoryStore, VectorMemory, VectorMemoryConfig};
pub use working::WorkingMemory;

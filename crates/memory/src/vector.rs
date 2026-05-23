// crates/memory/src/vector.rs
// Long-term vector memory backed by Qdrant with in-memory fallback.
use std::sync::Arc;

use agent_core::MemoryChunk;
use anyhow::Result;

use crate::embedding::Embedder;

/// Configuration for the vector memory store.
#[derive(Debug, Clone)]
pub struct VectorMemoryConfig {
    pub url: String,
    pub collection: String,
    pub embedding_dim: usize,
}

/// Long-term vector memory.
///
/// In production, connects to a Qdrant instance for persistent semantic storage.
/// Falls back to an in-memory store when Qdrant is unavailable (dev/test).
pub struct VectorMemory {
    embedder: Arc<dyn Embedder>,
    collection: String,
    /// In-memory fallback store: embedding -> chunk.
    entries: parking_lot::RwLock<Vec<(Vec<f32>, MemoryChunk)>>,
    /// Optional Qdrant client for production use.
    #[allow(dead_code)]
    qdrant_url: String,
    qdrant_client: Option<reqwest::Client>,
}

impl VectorMemory {
    /// Create a new VectorMemory, attempting to connect to Qdrant.
    /// Creates an internal VoyageEmbedder from the config's embedding_dim.
    /// Falls back to in-memory storage if Qdrant is not reachable.
    pub async fn new(cfg: &VectorMemoryConfig) -> Result<Self> {
        let embedder = Arc::new(crate::embedding::VoyageEmbedder::new(cfg.embedding_dim));
        let client = reqwest::Client::new();
        // Try to connect to Qdrant — if it fails, we fall back to in-memory
        let qdrant_available = client
            .get(format!("{}/health", cfg.url))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "Qdrant health check failed — using in-memory fallback");
                false
            });

        if qdrant_available {
            tracing::info!("Connected to Qdrant at {}", cfg.url);
        } else {
            tracing::info!(
                "Qdrant not available at {} — using in-memory fallback",
                cfg.url
            );
        }

        Ok(Self {
            embedder,
            collection: cfg.collection.clone(),
            entries: parking_lot::RwLock::new(Vec::new()),
            qdrant_url: cfg.url.clone(),
            qdrant_client: if qdrant_available {
                Some(client)
            } else {
                None
            },
        })
    }

    /// Executes a semantic search using the embedder and store backends.
    async fn do_search(&self, query: &str, k: usize) -> Result<Vec<MemoryChunk>> {
        let query_vec = self.embedder.embed(query).await?;

        // Try Qdrant first
        if let Some(ref client) = self.qdrant_client {
            match self.search_qdrant(client, &query_vec, k).await {
                Ok(results) if !results.is_empty() => return Ok(results),
                Ok(_) => {} // empty results, fall through to in-memory
                Err(e) => {
                    tracing::warn!("Qdrant search failed: {e}, falling back to in-memory");
                }
            }
        }

        // In-memory fallback
        self.search_in_memory(&query_vec, k)
    }

    /// Executes an upsert with embedding and backend dispatch.
    async fn do_upsert(&self, chunk: MemoryChunk) -> Result<()> {
        let embedding = self.embedder.embed(&chunk.content).await?;
        let mut chunk = chunk;
        chunk.embedding = embedding.clone();

        // Try Qdrant first
        if let Some(ref client) = self.qdrant_client {
            if let Err(e) = self.upsert_qdrant(client, &chunk, &embedding).await {
                tracing::warn!("Qdrant upsert failed: {e}, falling back to in-memory");
            } else {
                return Ok(());
            }
        }

        // In-memory fallback
        let mut entries = self.entries.write();
        entries.retain(|(_, c)| c.id != chunk.id);
        entries.push((embedding, chunk));
        Ok(())
    }

    /// Qdrant search via REST API.
    async fn search_qdrant(
        &self,
        client: &reqwest::Client,
        vector: &[f32],
        k: usize,
    ) -> Result<Vec<MemoryChunk>> {
        let url = format!(
            "{}/collections/{}/points/search",
            self.qdrant_url, self.collection
        );
        let body = serde_json::json!({
            "vector": vector,
            "limit": k,
            "with_payload": true
        });

        let resp = client
            .post(&url)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(anyhow::anyhow!("Qdrant search returned {}", resp.status()));
        }

        let data: serde_json::Value = resp.json().await?;
        let results = data["result"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|point| {
                        let id_str = point["id"].as_str().or_else(|| point["id"].as_str())?;
                        let content = point["payload"]["content"].as_str()?;
                        let embedding: Vec<f32> = point["vector"]
                            .as_array()?
                            .iter()
                            .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                            .collect();
                        Some(MemoryChunk {
                            id: uuid::Uuid::parse_str(id_str).unwrap_or_else(|e| {
                                tracing::warn!(id_str = id_str, error = %e, "Qdrant returned invalid point UUID, generating new ID");
                                uuid::Uuid::new_v4()
                            }),
                            content: content.to_string(),
                            embedding,
                            timestamp: chrono::Utc::now(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(results)
    }

    /// Qdrant upsert via REST API.
    async fn upsert_qdrant(
        &self,
        client: &reqwest::Client,
        chunk: &MemoryChunk,
        vector: &[f32],
    ) -> Result<()> {
        let url = format!(
            "{}/collections/{}/points",
            self.qdrant_url, self.collection
        );
        let body = serde_json::json!({
            "points": [{
                "id": chunk.id.to_string(),
                "vector": vector,
                "payload": {
                    "content": chunk.content,
                    "timestamp": chunk.timestamp.to_rfc3339()
                }
            }]
        });

        let resp = client
            .put(&url)
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let err_body = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Qdrant upsert returned {status}: {err_body}"));
        }

        Ok(())
    }

    /// In-memory cosine similarity search.
    fn search_in_memory(&self, query_vec: &[f32], k: usize) -> Result<Vec<MemoryChunk>> {
        let entries = self.entries.read();
        let mut scored: Vec<(f64, MemoryChunk)> = entries
            .iter()
            .map(|(emb, chunk)| {
                let sim = cosine_similarity(query_vec, emb);
                (sim, chunk.clone())
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        Ok(scored.into_iter().take(k).map(|(_, chunk)| chunk).collect())
    }
}

/// Mock memory store for testing — stores everything in memory.
pub struct MockMemoryStore {
    entries: parking_lot::RwLock<Vec<MemoryChunk>>,
}

impl MockMemoryStore {
    pub fn new() -> Self {
        Self {
            entries: parking_lot::RwLock::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl agent_core::MemoryStore for MockMemoryStore {
    async fn upsert(&self, chunk: MemoryChunk) -> anyhow::Result<()> {
        let mut entries = self.entries.write();
        entries.retain(|c| c.id != chunk.id);
        entries.push(chunk);
        Ok(())
    }

    async fn search(&self, _query: &str, _k: usize) -> anyhow::Result<Vec<MemoryChunk>> {
        Ok(self.entries.read().clone())
    }
}

#[async_trait::async_trait]
impl agent_core::MemoryStore for VectorMemory {
    async fn upsert(&self, chunk: MemoryChunk) -> anyhow::Result<()> {
        self.do_upsert(chunk).await
    }

    async fn search(&self, query: &str, k: usize) -> anyhow::Result<Vec<MemoryChunk>> {
        self.do_search(query, k).await
    }
}

/// Cosine similarity between two vectors.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    let dot: f64 = a.iter().zip(b).map(|(x, y)| *x as f64 * *y as f64).sum();
    let norm_a: f64 = a.iter().map(|x| *x as f64 * *x as f64).sum::<f64>().sqrt();
    let norm_b: f64 = b.iter().map(|x| *x as f64 * *x as f64).sum::<f64>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::MemoryStore;

    #[test]
    fn test_cosine_similarity_identical() {
        let v = vec![1.0_f32, 2.0, 3.0];
        let sim = cosine_similarity(&v, &v);
        assert!((sim - 1.0).abs() < 0.001, "identical vectors should have similarity 1.0, got {sim}");
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0_f32, 0.0];
        let b = vec![0.0_f32, 1.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 0.0).abs() < 0.001, "orthogonal vectors should have similarity 0.0, got {sim}");
    }

    #[test]
    fn test_cosine_similarity_zero_vector() {
        let a = vec![0.0_f32; 4];
        let b = vec![1.0_f32; 4];
        let sim = cosine_similarity(&a, &b);
        assert_eq!(sim, 0.0);
    }

    #[tokio::test]
    async fn test_mock_memory_store_upsert_and_search() {
        let store = MockMemoryStore::new();
        let chunk = MemoryChunk {
            id: uuid::Uuid::new_v4(),
            content: "hello world".into(),
            embedding: vec![],
            timestamp: chrono::Utc::now(),
        };
        store.upsert(chunk.clone()).await.unwrap();
        let results = store.search("hello", 5).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content, "hello world");
    }

    #[tokio::test]
    async fn test_mock_memory_store_dedup_by_id() {
        let store = MockMemoryStore::new();
        let id = uuid::Uuid::new_v4();
        let chunk1 = MemoryChunk {
            id,
            content: "first".into(),
            embedding: vec![],
            timestamp: chrono::Utc::now(),
        };
        let chunk2 = MemoryChunk {
            id,
            content: "second".into(),
            embedding: vec![],
            timestamp: chrono::Utc::now(),
        };
        store.upsert(chunk1).await.unwrap();
        store.upsert(chunk2).await.unwrap();
        let results = store.search("", 5).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content, "second");
    }
}

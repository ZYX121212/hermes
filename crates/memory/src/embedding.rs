// crates/memory/src/embedding.rs
use async_trait::async_trait;

/// Trait for text-to-vector embedding.
#[async_trait]
pub trait Embedder: Send + Sync {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>>;
    fn dimension(&self) -> usize;
}

/// Voyage AI embedder — calls the Voyage API when a key is configured,
/// falls back to zero vectors otherwise.
pub struct VoyageEmbedder {
    dimension: usize,
    api_key: Option<String>,
    client: reqwest::Client,
}

impl VoyageEmbedder {
    pub fn new(dimension: usize) -> Self {
        Self {
            dimension,
            api_key: std::env::var("VOYAGE_API_KEY").ok(),
            client: reqwest::Client::new(),
        }
    }

    pub fn with_api_key(dimension: usize, api_key: String) -> Self {
        Self {
            dimension,
            api_key: Some(api_key),
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl Embedder for VoyageEmbedder {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let Some(ref api_key) = self.api_key else {
            use std::sync::atomic::{AtomicBool, Ordering};
            static EMBED_WARNED: AtomicBool = AtomicBool::new(false);
            if !EMBED_WARNED.swap(true, Ordering::Relaxed) {
                tracing::warn!("VoyageEmbedder: no VOYAGE_API_KEY set, using zero vector embeddings (semantic search disabled)");
            }
            return Ok(vec![0.0_f32; self.dimension]);
        };

        let resp = self
            .client
            .post("https://api.voyageai.com/v1/embeddings")
            .header("Authorization", format!("Bearer {api_key}"))
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "model": "voyage-3",
                "input": text
            }))
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err = resp.text().await.unwrap_or_default();
            tracing::warn!(status = %status, error = %err, "Voyage API error, falling back to zero vector");
            return Ok(vec![0.0_f32; self.dimension]);
        }

        let body: serde_json::Value = resp.json().await?;
        let embedding: Vec<f32> = body["data"][0]["embedding"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                    .collect()
            })
            .unwrap_or_default();

        Ok(embedding)
    }

    fn dimension(&self) -> usize {
        self.dimension
    }
}

/// Simple hash-based embedder for testing without API calls.
pub struct HashEmbedder {
    dimension: usize,
}

impl HashEmbedder {
    pub fn new(dimension: usize) -> Self {
        Self { dimension }
    }
}

#[async_trait]
impl Embedder for HashEmbedder {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut out = vec![0.0_f32; self.dimension];
        for (i, byte) in text.bytes().enumerate() {
            let mut hasher = DefaultHasher::new();
            (i, byte).hash(&mut hasher);
            let h = hasher.finish();
            out[i % self.dimension] = (h as f32 / u64::MAX as f32) * 2.0 - 1.0;
        }
        Ok(out)
    }

    fn dimension(&self) -> usize {
        self.dimension
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_hash_embedder_deterministic() {
        let e = HashEmbedder::new(64);
        let v1 = e.embed("hello").await.unwrap();
        let v2 = e.embed("hello").await.unwrap();
        assert_eq!(v1, v2);
    }

    #[tokio::test]
    async fn test_hash_embedder_different_inputs() {
        let e = HashEmbedder::new(64);
        let v1 = e.embed("hello").await.unwrap();
        let v2 = e.embed("world").await.unwrap();
        assert_ne!(v1, v2);
    }

    #[test]
    fn test_dimension() {
        let e = HashEmbedder::new(128);
        assert_eq!(e.dimension(), 128);
    }

    #[tokio::test]
    async fn test_voyage_embedder_no_key_returns_zeros() {
        // Ensure no API key is set in the environment for this test
        let e = VoyageEmbedder {
            dimension: 16,
            api_key: None,
            client: reqwest::Client::new(),
        };
        let v = e.embed("test").await.unwrap();
        assert_eq!(v, vec![0.0_f32; 16]);
    }
}

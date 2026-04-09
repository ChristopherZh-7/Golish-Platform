//! Embedding generation for semantic memory.
//!
//! Provides an [`Embedder`] trait and a [`HttpEmbedder`] implementation that
//! calls any OpenAI-compatible embedding endpoint (OpenAI, Azure, local Ollama, etc.).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Trait for generating text embeddings.
#[async_trait::async_trait]
pub trait Embedder: Send + Sync {
    /// Embed a single text string into a vector.
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;

    /// Embed multiple texts in a single batch call (default: sequential fallback).
    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let mut results = Vec::with_capacity(texts.len());
        for text in texts {
            results.push(self.embed(text).await?);
        }
        Ok(results)
    }

    /// The dimensionality of embeddings produced by this model.
    fn dimension(&self) -> usize;

    /// A human-readable name for logging.
    fn model_name(&self) -> &str;
}

/// Calls any OpenAI-compatible `/v1/embeddings` endpoint.
pub struct HttpEmbedder {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    dim: usize,
}

impl HttpEmbedder {
    pub fn new(base_url: &str, api_key: &str, model: &str, dim: usize) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            dim,
        }
    }

    /// Convenience constructor for OpenAI's text-embedding-3-small (1536-dim).
    pub fn openai_small(api_key: &str) -> Self {
        Self::new(
            "https://api.openai.com/v1",
            api_key,
            "text-embedding-3-small",
            1536,
        )
    }

    /// Convenience constructor for OpenAI's text-embedding-3-large (3072-dim).
    pub fn openai_large(api_key: &str) -> Self {
        Self::new(
            "https://api.openai.com/v1",
            api_key,
            "text-embedding-3-large",
            3072,
        )
    }

    /// Convenience constructor for a local Ollama server.
    pub fn ollama(model: &str, dim: usize) -> Self {
        Self::new("http://localhost:11434/v1", "", model, dim)
    }
}

#[derive(Serialize)]
struct EmbeddingRequest<'a> {
    model: &'a str,
    input: &'a [&'a str],
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingObject>,
}

#[derive(Deserialize)]
struct EmbeddingObject {
    embedding: Vec<f32>,
}

#[async_trait::async_trait]
impl Embedder for HttpEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let results = self.embed_batch(&[text]).await?;
        results
            .into_iter()
            .next()
            .context("empty embedding response")
    }

    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let url = format!("{}/embeddings", self.base_url);
        let body = EmbeddingRequest {
            model: &self.model,
            input: texts,
        };

        let mut req = self.client.post(&url).json(&body);
        if !self.api_key.is_empty() {
            req = req.bearer_auth(&self.api_key);
        }

        let resp = req
            .send()
            .await
            .context("embedding API request failed")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("embedding API returned {status}: {body}");
        }

        let parsed: EmbeddingResponse = resp
            .json()
            .await
            .context("failed to parse embedding response")?;

        Ok(parsed.data.into_iter().map(|o| o.embedding).collect())
    }

    fn dimension(&self) -> usize {
        self.dim
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}

/// A no-op embedder that returns zero vectors. Useful for testing
/// or when no embedding API is configured.
pub struct NoopEmbedder {
    dim: usize,
}

impl NoopEmbedder {
    pub fn new(dim: usize) -> Self {
        Self { dim }
    }
}

#[async_trait::async_trait]
impl Embedder for NoopEmbedder {
    async fn embed(&self, _text: &str) -> Result<Vec<f32>> {
        Ok(vec![0.0; self.dim])
    }

    fn dimension(&self) -> usize {
        self.dim
    }

    fn model_name(&self) -> &str {
        "noop"
    }
}

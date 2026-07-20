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

    /// Construct a local-only OpenAI-compatible embedding client. The endpoint
    /// must be an HTTP loopback literal; redirects and environment proxies are
    /// disabled so customer text cannot leave the host through proxy policy.
    pub fn local_openai_compatible(base_url: &str, model: &str, dim: usize) -> Result<Self> {
        let mut url = reqwest::Url::parse(base_url).context("invalid local embedding URL")?;
        anyhow::ensure!(url.scheme() == "http", "local embedding URL must use http");
        anyhow::ensure!(
            matches!(url.host_str(), Some("127.0.0.1") | Some("::1")),
            "local embedding URL must use a loopback IP literal"
        );
        anyhow::ensure!(
            url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "local embedding URL cannot contain credentials, query, or fragment"
        );
        match url.path().trim_end_matches('/') {
            "" => url.set_path("/v1"),
            "/v1" => url.set_path("/v1"),
            _ => anyhow::bail!("local embedding URL path must be / or /v1"),
        }
        let model = model.trim();
        anyhow::ensure!(!model.is_empty(), "local embedding model is empty");
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .context("build local embedding client")?;
        Ok(Self {
            client,
            base_url: url.as_str().trim_end_matches('/').to_string(),
            api_key: String::new(),
            model: model.to_string(),
            dim,
        })
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
    dimensions: usize,
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
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let url = format!("{}/embeddings", self.base_url);
        let body = EmbeddingRequest {
            model: &self.model,
            input: texts,
            dimensions: self.dim,
        };

        let mut req = self.client.post(&url).json(&body);
        if !self.api_key.is_empty() {
            req = req.bearer_auth(&self.api_key);
        }

        let resp = req.send().await.context("embedding API request failed")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("embedding API returned {status}: {body}");
        }

        let parsed: EmbeddingResponse = resp
            .json()
            .await
            .context("failed to parse embedding response")?;

        anyhow::ensure!(
            parsed.data.len() == texts.len(),
            "embedding API response count mismatch"
        );
        parsed
            .data
            .into_iter()
            .map(|object| {
                anyhow::ensure!(
                    object.embedding.len() == self.dim,
                    "embedding API response dimension mismatch"
                );
                anyhow::ensure!(
                    object.embedding.iter().all(|value| value.is_finite()),
                    "embedding API response contains non-finite values"
                );
                Ok(object.embedding)
            })
            .collect()
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

#[cfg(test)]
mod tests {
    use super::{Embedder, HttpEmbedder};

    #[test]
    fn local_embedder_accepts_only_loopback_v1_and_fixed_identity() {
        let embedder = HttpEmbedder::local_openai_compatible(
            "http://127.0.0.1:11434",
            "qwen3-embedding:4b",
            1536,
        )
        .expect("loopback Ollama endpoint is accepted");
        assert_eq!(embedder.base_url, "http://127.0.0.1:11434/v1");
        assert_eq!(embedder.dimension(), 1536);
        assert_eq!(embedder.model_name(), "qwen3-embedding:4b");

        for rejected in [
            "https://127.0.0.1:11434/v1",
            "http://localhost:11434/v1",
            "http://192.168.1.5:11434/v1",
            "http://127.0.0.1:11434/api",
            "http://user@127.0.0.1:11434/v1",
            "http://127.0.0.1:11434/v1?proxy=true",
        ] {
            assert!(
                HttpEmbedder::local_openai_compatible(rejected, "model", 1536).is_err(),
                "unsafe local endpoint must be rejected: {rejected}"
            );
        }
    }
}

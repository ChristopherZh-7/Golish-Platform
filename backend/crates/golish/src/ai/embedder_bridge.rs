//! Bridge between `golish_db::embeddings::Embedder` and
//! `golish_agent_kit::db_traits::TextEmbedder`.

use async_trait::async_trait;

pub struct EmbedderBridge<E: golish_db::embeddings::Embedder> {
    inner: E,
}

impl<E: golish_db::embeddings::Embedder> EmbedderBridge<E> {
    pub fn new(inner: E) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl<E: golish_db::embeddings::Embedder> golish_agent_kit::db_traits::TextEmbedder
    for EmbedderBridge<E>
{
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        self.inner.embed(text).await
    }

    async fn embed_batch(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        self.inner.embed_batch(texts).await
    }

    fn dimension(&self) -> usize {
        self.inner.dimension()
    }

    fn model_name(&self) -> &str {
        self.inner.model_name()
    }
}

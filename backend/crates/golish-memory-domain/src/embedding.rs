/// Memory/RAG embedding schema V1 is intentionally fixed. A different
/// dimension requires an additive physical schema/version, never truncation or
/// zero padding.
pub const EMBEDDING_DIMENSION_V1: usize = 1536;

pub fn validate_embedding_dimension(dimension: usize) -> Result<(), EmbeddingDimensionError> {
    if dimension == EMBEDDING_DIMENSION_V1 {
        Ok(())
    } else {
        Err(EmbeddingDimensionError {
            expected: EMBEDDING_DIMENSION_V1,
            actual: dimension,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("embedding dimension mismatch: expected {expected}, got {actual}")]
pub struct EmbeddingDimensionError {
    pub expected: usize,
    pub actual: usize,
}

impl EmbeddingDimensionError {
    pub const fn code(self) -> &'static str {
        "memory_embedding_dimension_mismatch"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_dimension_fails_closed() {
        assert!(validate_embedding_dimension(EMBEDDING_DIMENSION_V1).is_ok());
        let error = validate_embedding_dimension(1024).expect_err("1024 must not be accepted");
        assert_eq!(error.code(), "memory_embedding_dimension_mismatch");
    }
}

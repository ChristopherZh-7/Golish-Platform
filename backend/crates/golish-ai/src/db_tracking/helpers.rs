use golish_db::DbReadyGate;

/// Convert a Vec<f32> into pgvector's text format: `[0.1,0.2,...]`
pub(super) fn vec_to_pgvector(v: &[f32]) -> String {
    let parts: Vec<String> = v.iter().map(|f| f.to_string()).collect();
    format!("[{}]", parts.join(","))
}

pub(super) fn truncate_for_db(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_bytes).collect();
        format!("{}... [truncated]", truncated)
    }
}

/// Wait for PG to become ready with a 60-second timeout.
/// Returns `true` if PG is ready, `false` if timed out or failed (caller should skip the write).
pub(super) async fn await_db_ready(gate: &mut DbReadyGate) -> bool {
    if gate.is_ready() {
        return true;
    }
    if gate.is_failed() {
        return false;
    }
    match tokio::time::timeout(std::time::Duration::from_secs(60), gate.wait()).await {
        Ok(ready) => ready,
        Err(_) => {
            tracing::warn!("[db-track] Timed out waiting for PostgreSQL readiness, skipping write");
            false
        }
    }
}

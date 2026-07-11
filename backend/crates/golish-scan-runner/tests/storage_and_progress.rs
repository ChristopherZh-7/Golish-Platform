//! Tests for the trait surface and pure helpers in golish-scan-runner.
//!
//! We exercise the `ScanStorage` trait via a mock implementation and verify
//! that the global `NUCLEI_CANCELLED` cancellation flag round-trips through
//! the public API. Network/process-spawning paths require external tools and
//! are intentionally excluded.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use sqlx::PgPool;
use uuid::Uuid;

use golish_db::repo::scoped::TargetWriteGuard;
use golish_scan_runner::{ScanRunnerResult, ScanStorage, NUCLEI_CANCELLED};

#[derive(Default)]
struct MockScanStorage {
    calls: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl ScanStorage for MockScanStorage {
    async fn store_directory_entry(
        &self,
        _pool: &PgPool,
        _guard: &TargetWriteGuard,
        url: &str,
        status_code: Option<i32>,
        _content_length: Option<i32>,
        _lines: Option<i32>,
        _words: Option<i32>,
        tool: &str,
    ) -> ScanRunnerResult<()> {
        self.calls.lock().push(format!(
            "{}|{}|{}",
            tool,
            url,
            status_code
                .map(|s| s.to_string())
                .unwrap_or_else(|| "?".into())
        ));
        Ok(())
    }
}

fn guard() -> TargetWriteGuard {
    TargetWriteGuard {
        target_id: Uuid::new_v4(),
        organization_id: Some(Uuid::new_v4()),
        project_path: "/workspace".to_string(),
        scope: "in".to_string(),
        name: "https://example.com/".to_string(),
        value: "https://example.com/".to_string(),
        ports: serde_json::json!([]),
    }
}

#[tokio::test]
async fn mock_storage_records_directory_entry() {
    // We pass a default-constructed PgPool reference via std::ptr::null is unsound,
    // so we build the call against the trait through dispatch and skip the pool
    // by using the mock which ignores it. This requires a real PgPool just for
    // the type signature — we use lazy-connect which never opens a connection
    // because the mock never queries.
    let pool = PgPool::connect_lazy("postgres://nobody:nobody@127.0.0.1/none")
        .expect("lazy connect builds without contacting server");

    let mock = MockScanStorage::default();
    let calls = mock.calls.clone();
    let storage: &dyn ScanStorage = &mock;
    let guard = guard();

    storage
        .store_directory_entry(
            &pool,
            &guard,
            "https://example.com/admin",
            Some(200),
            Some(1024),
            Some(50),
            Some(120),
            "feroxbuster",
        )
        .await
        .expect("mock cannot fail");

    let recorded = calls.lock().clone();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0], "feroxbuster|https://example.com/admin|200");
}

#[test]
fn nuclei_cancellation_flag_round_trips() {
    // Ensure clean baseline (other tests might have flipped it).
    NUCLEI_CANCELLED.store(false, Ordering::SeqCst);
    assert!(!NUCLEI_CANCELLED.load(Ordering::SeqCst));

    NUCLEI_CANCELLED.store(true, Ordering::SeqCst);
    assert!(NUCLEI_CANCELLED.load(Ordering::SeqCst));

    NUCLEI_CANCELLED.store(false, Ordering::SeqCst);
    assert!(!NUCLEI_CANCELLED.load(Ordering::SeqCst));
}

#[tokio::test]
async fn mock_storage_records_multiple_calls_in_order() {
    let pool = PgPool::connect_lazy("postgres://nobody:nobody@127.0.0.1/none").unwrap();
    let mock = MockScanStorage::default();
    let calls = mock.calls.clone();
    let storage: &dyn ScanStorage = &mock;
    let guard = guard();

    for (i, url) in ["a", "b", "c"].iter().enumerate() {
        storage
            .store_directory_entry(
                &pool,
                &guard,
                url,
                Some(200 + i as i32),
                None,
                None,
                None,
                "ferox",
            )
            .await
            .unwrap();
    }

    let recorded = calls.lock().clone();
    assert_eq!(recorded.len(), 3);
    assert!(recorded[0].ends_with("|200"));
    assert!(recorded[1].ends_with("|201"));
    assert!(recorded[2].ends_with("|202"));
}

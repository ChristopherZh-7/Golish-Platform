//! Authorization boundary shared by the legacy GUI scan runners.
//!
//! A scan is allowed to launch only for one immutable, current in-scope target
//! witness and one exact HTTP(S) Web Origin owned by that target.  The witness
//! is captured before asynchronous preparation and revalidated immediately
//! before every child-process launch; target-bound result writers receive the
//! same witness for their short guarded transactions.

use std::collections::BTreeSet;
use std::future::Future;

use golish_db::repo::scoped::TargetWriteGuard;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{ScanRunnerError, ScanRunnerResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizedScanTarget {
    pub guard: TargetWriteGuard,
    pub requested_url: String,
    pub exact_origin: String,
}

/// Load and bind one caller request before any asynchronous tool lookup or
/// command preparation.  A missing project path is never treated as a global
/// workspace, and the caller-provided project must match the DB witness byte
/// for byte.
pub async fn authorize_scan_target(
    pool: &PgPool,
    target_id: Uuid,
    caller_project_path: Option<&str>,
    requested_url: &str,
) -> ScanRunnerResult<AuthorizedScanTarget> {
    let guard = golish_db::repo::scoped::load_target_write_guard(pool, target_id)
        .await?
        .ok_or_else(|| {
            ScanRunnerError::Other(anyhow::anyhow!(
                "scan target must be current, in scope, and bound to a project"
            ))
        })?;
    authorize_scan_target_from_guard(guard, caller_project_path, requested_url)
}

pub fn authorize_scan_target_from_guard(
    guard: TargetWriteGuard,
    caller_project_path: Option<&str>,
    requested_url: &str,
) -> ScanRunnerResult<AuthorizedScanTarget> {
    if guard.scope != "in" {
        return Err(ScanRunnerError::Other(anyhow::anyhow!(
            "scan target is out of scope"
        )));
    }
    let caller_project_path = caller_project_path
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ScanRunnerError::Other(anyhow::anyhow!(
                "scan launch requires the caller project_path"
            ))
        })?;
    if caller_project_path != guard.project_path {
        return Err(ScanRunnerError::Other(anyhow::anyhow!(
            "caller project_path does not match the target owner"
        )));
    }

    let requested_url = requested_url.trim();
    let requested_origin = canonical_origin(requested_url).ok_or_else(|| {
        ScanRunnerError::Other(anyhow::anyhow!(
            "scan target_url must be an absolute HTTP(S) URL without credentials"
        ))
    })?;
    if !authorized_origins(&guard).contains(&requested_origin) {
        return Err(ScanRunnerError::Other(anyhow::anyhow!(
            "scan target_url exact origin is not owned by target name/value or a confirmed-open ports[].url"
        )));
    }

    Ok(AuthorizedScanTarget {
        guard,
        requested_url: requested_url.to_string(),
        exact_origin: requested_origin,
    })
}

pub fn url_has_authorized_origin(authorization: &AuthorizedScanTarget, value: &str) -> bool {
    canonical_origin(value).as_deref() == Some(authorization.exact_origin.as_str())
}

fn canonical_origin(value: &str) -> Option<String> {
    golish_pentest_domain::canonical_web_origin(value).map(|origin| origin.key)
}

fn authorized_origins(guard: &TargetWriteGuard) -> BTreeSet<String> {
    let mut origins = BTreeSet::new();
    for value in [&guard.name, &guard.value] {
        if let Some(origin) = canonical_origin(value) {
            origins.insert(origin);
        }
    }
    for port in guard.ports.as_array().into_iter().flatten() {
        let confirmed_open = port
            .get("state")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|state| state.trim().eq_ignore_ascii_case("open"));
        if !confirmed_open {
            continue;
        }
        if let Some(origin) = port
            .get("url")
            .and_then(serde_json::Value::as_str)
            .and_then(canonical_origin)
        {
            origins.insert(origin);
        }
    }
    origins
}

/// Execute a command-launch closure only after its immediately preceding
/// validator succeeds.  Production passes DB guard validation as `validation`;
/// tests pass a synthetic owner-drift error and a counting fake launcher.
pub(crate) async fn after_successful_validation<T, E, ValidationFuture, Launch, LaunchFuture>(
    validation: ValidationFuture,
    launch: Launch,
) -> Result<T, E>
where
    ValidationFuture: Future<Output = Result<(), E>>,
    Launch: FnOnce() -> LaunchFuture,
    LaunchFuture: Future<Output = Result<T, E>>,
{
    validation.await?;
    launch().await
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::*;

    fn guard() -> TargetWriteGuard {
        TargetWriteGuard {
            target_id: Uuid::new_v4(),
            organization_id: Some(Uuid::new_v4()),
            project_path: "/workspace/a".to_string(),
            scope: "in".to_string(),
            name: "https://app.example/".to_string(),
            value: "app.example".to_string(),
            ports: serde_json::json!([
                {"port": 8443, "state": "open", "url": "https://app.example:8443/"},
                {"port": 9443, "state": "closed", "url": "https://app.example:9443/"}
            ]),
        }
    }

    #[test]
    fn launch_authorization_rejects_foreign_origin_project_and_unconfirmed_port() {
        assert!(authorize_scan_target_from_guard(
            guard(),
            Some("/workspace/a"),
            "https://app.example:8443/admin"
        )
        .is_ok());
        assert!(authorize_scan_target_from_guard(
            guard(),
            Some("/workspace/b"),
            "https://app.example/"
        )
        .is_err());
        assert!(authorize_scan_target_from_guard(
            guard(),
            Some("/workspace/a"),
            "https://foreign.example/"
        )
        .is_err());
        assert!(authorize_scan_target_from_guard(
            guard(),
            Some("/workspace/a"),
            "https://app.example:9443/"
        )
        .is_err());
    }

    #[tokio::test]
    async fn owner_drift_validation_prevents_fake_command_launch() {
        let launches = Arc::new(AtomicUsize::new(0));
        let fake_launches = Arc::clone(&launches);
        let result: Result<(), &'static str> =
            after_successful_validation(async { Err("target owner drift") }, move || async move {
                fake_launches.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .await;

        assert_eq!(result, Err("target owner drift"));
        assert_eq!(launches.load(Ordering::SeqCst), 0);
    }
}

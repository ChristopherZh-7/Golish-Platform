//! Operation-bound target authorization for the passive-intel -> active-recon
//! boundary. Provider discoveries are presented to the human once; only an
//! unchanged non-empty subset becomes trusted active scope.

use std::collections::BTreeSet;

use uuid::Uuid;

use crate::db_traits::{ActiveReconScopeReviewApproval, ScopingReviewedTarget};
use crate::task_orchestrator::TaskOrchestrator;
use golish_core::events::AiEvent;

pub(super) const ACTIVE_RECON_TRUSTED_TARGET_REQUIRED: &str =
    "ACTIVE_RECON_TRUSTED_TARGET_REQUIRED";
const ACTIVE_RECON_SCOPE_REVIEW_TIMEOUT_SECS: u64 = 600;

impl TaskOrchestrator {
    /// TargetIntel -> EAS's single human boundary. Existing exact intake is
    /// already authorized; otherwise the current Target Intel candidates are
    /// shown in a scope-review table and the accepted subset is frozen before
    /// the caller advances. Success intentionally replaces the generic phase
    /// approval at this exact crossing.
    pub(super) async fn ensure_active_recon_target_scope(&mut self, task_id: Uuid) -> bool {
        let Some(organization_id) = self.harness_org_id else {
            self.emit_active_recon_target_hold(
                task_id,
                "the operation has no trusted engagement organization",
                "waiting_target_scope",
            );
            return false;
        };

        let presented = match self
            .repo
            .active_recon_scope_review_candidates(task_id, organization_id)
            .await
        {
            Ok(rows) if rows.is_empty() => {
                return match self.active_recon_target_authority(task_id).await {
                    Ok(true) => true,
                    Ok(false) => {
                        self.emit_active_recon_target_hold(
                            task_id,
                            "Target Intel produced no valid exact provider-derived targets to review",
                            "waiting_target_scope",
                        );
                        false
                    }
                    Err(detail) => {
                        self.emit_active_recon_target_hold(
                            task_id,
                            &detail,
                            "waiting_target_scope",
                        );
                        false
                    }
                };
            }
            Ok(rows) if valid_review_rows(&rows, false) => rows,
            Ok(_) => {
                self.emit_active_recon_target_hold(
                    task_id,
                    "the active target denominator contains invalid or duplicate rows",
                    "waiting_target_scope",
                );
                return false;
            }
            Err(error) => {
                tracing::warn!(
                    target: "harness::hook",
                    task_id = %task_id,
                    organization_id = %organization_id,
                    error = %error,
                    "active-recon review candidate lookup failed"
                );
                self.emit_active_recon_target_hold(
                    task_id,
                    "the Target Intel candidate snapshot could not be verified",
                    "waiting_target_scope",
                );
                return false;
            }
        };
        let Some(coordinator) = self.approval_coordinator.clone() else {
            self.emit_active_recon_target_hold(
                task_id,
                "interactive target review is unavailable; add an exact target through trusted UI/CLI intake and rerun Scoping",
                "waiting_target_scope",
            );
            return false;
        };

        let context = match serde_json::to_string(&presented) {
            Ok(context) => context,
            Err(error) => {
                tracing::warn!(
                    target: "harness::hook",
                    task_id = %task_id,
                    error = %error,
                    "active-recon review candidates could not be serialized"
                );
                return false;
            }
        };
        let request_id = Uuid::new_v4().to_string();
        let decision_rx = coordinator.register_approval(request_id.clone());
        self.emit(AiEvent::TaskProgress {
            task_id: task_id.to_string(),
            status: "waiting_target_scope".to_string(),
            message: "Review the exact targets that may enter active recon. Remove any target that is not authorized; editing or adding targets is rejected.".to_string(),
        });
        self.emit(AiEvent::AskHumanRequest {
            request_id: request_id.clone(),
            question: "Confirm the exact target subset authorized for active reconnaissance?"
                .to_string(),
            input_type: "scope_review".to_string(),
            options: Vec::new(),
            context,
        });

        let decision = match tokio::time::timeout(
            std::time::Duration::from_secs(ACTIVE_RECON_SCOPE_REVIEW_TIMEOUT_SECS),
            decision_rx,
        )
        .await
        {
            Ok(Ok(decision)) if decision.approved => decision,
            _ => {
                self.emit_active_recon_target_hold(
                    task_id,
                    "the target scope review was skipped, declined, or timed out",
                    "waiting_target_scope",
                );
                return false;
            }
        };
        let Some(selected) = decision
            .reason
            .as_deref()
            .and_then(parse_scope_review_response)
            .filter(|rows| unchanged_nonempty_subset(&presented, rows))
        else {
            self.emit_active_recon_target_hold(
                task_id,
                "the target review must return a non-empty unchanged subset of the presented rows",
                "waiting_target_scope",
            );
            return false;
        };

        let approval = ActiveReconScopeReviewApproval {
            request_id,
            presented,
            selected,
        };
        let applied = match self
            .repo
            .active_recon_scope_review_apply(task_id, organization_id, approval)
            .await
        {
            Ok(rows) if valid_review_rows(&rows, false) => rows,
            Ok(_) => {
                self.emit_active_recon_target_hold(
                    task_id,
                    "the persisted target authorization is empty or invalid",
                    "waiting_target_scope",
                );
                return false;
            }
            Err(error) => {
                tracing::warn!(
                    target: "harness::hook",
                    task_id = %task_id,
                    organization_id = %organization_id,
                    error = %error,
                    "active-recon target review transaction failed"
                );
                self.emit_active_recon_target_hold(
                    task_id,
                    "the target authorization could not be persisted atomically",
                    "waiting_target_scope",
                );
                return false;
            }
        };
        self.current_invocation_target_authority = Some(true);
        self.emit(AiEvent::TaskProgress {
            task_id: task_id.to_string(),
            status: "running".to_string(),
            message: format!(
                "Authorized {} exact target(s); entering active reconnaissance.",
                applied.len()
            ),
        });
        true
    }

    async fn active_recon_target_authority(&self, task_id: Uuid) -> Result<bool, String> {
        let organization_id = self
            .harness_org_id
            .ok_or_else(|| "the operation has no trusted engagement organization".to_string())?;

        if self.current_invocation_target_authority == Some(false) {
            return match self
                .repo
                .active_recon_scope_review_authorized(task_id, organization_id)
                .await
            {
                Ok(true) => Ok(true),
                Ok(false) => Err("the current CLI invocation supplied no exact target and no matching operation-bound target review exists".to_string()),
                Err(error) => {
                    tracing::warn!(
                        target: "harness::hook",
                        task_id = %task_id,
                        organization_id = %organization_id,
                        error = %error,
                        "operation-bound target authorization lookup failed"
                    );
                    Err("the current CLI invocation supplied no exact target and no matching operation-bound target review could be verified".to_string())
                }
            };
        }

        self.repo
            .scoping_target_snapshot(organization_id)
            .await
            .map(|snapshot| {
                snapshot.iter().any(|target| {
                    target.scope.trim().eq_ignore_ascii_case("in")
                        && canonical_scoping_target(target).is_some()
                })
            })
            .map_err(|error| {
                tracing::warn!(
                    target: "harness::hook",
                    task_id = %task_id,
                    organization_id = %organization_id,
                    error = %error,
                    "pre-EAS trusted target snapshot read failed; holding active recon"
                );
                "the trusted Scoping target snapshot could not be verified".to_string()
            })
    }

    fn emit_active_recon_target_hold(&self, task_id: Uuid, detail: &str, status: &str) {
        tracing::info!(
            target: "harness::hook",
            task_id = %task_id,
            reason = ACTIVE_RECON_TRUSTED_TARGET_REQUIRED,
            "pre-EAS trusted target barrier held the operation"
        );
        self.emit(AiEvent::TaskProgress {
            task_id: task_id.to_string(),
            status: status.to_string(),
            message: format!("{ACTIVE_RECON_TRUSTED_TARGET_REQUIRED}: {detail}."),
        });
    }
}

pub(crate) fn canonical_scoping_target(row: &ScopingReviewedTarget) -> Option<String> {
    let target_type = row.target_type.trim().to_ascii_lowercase();
    let scope = row.scope.trim().to_ascii_lowercase();
    if !matches!(scope.as_str(), "in" | "out") {
        return None;
    }
    let raw = row.value.trim();
    let identity = match target_type.as_str() {
        "url" => golish_pentest_domain::canonical_web_origin(raw)?.key,
        "wildcard" => {
            let base = raw.strip_prefix("*.")?;
            let key = golish_pentest_domain::canonical_asset_key(base)?;
            if key.class != golish_pentest_domain::AssetClass::Domain {
                return None;
            }
            format!("*.{}", key.key)
        }
        "domain" => {
            if raw.to_ascii_lowercase().starts_with("http://")
                || raw.to_ascii_lowercase().starts_with("https://")
                || raw.starts_with("*.")
            {
                return None;
            }
            let key = golish_pentest_domain::canonical_asset_key(raw)?;
            (key.class == golish_pentest_domain::AssetClass::Domain).then_some(key.key)?
        }
        "ip" => {
            let key = golish_pentest_domain::canonical_asset_key(raw)?;
            (key.class == golish_pentest_domain::AssetClass::Ip).then_some(key.key)?
        }
        "cidr" => canonical_scoping_cidr(raw)?,
        _ => return None,
    };
    Some(format!("{scope}|{target_type}|{identity}"))
}

pub(crate) fn canonical_scoping_cidr(raw: &str) -> Option<String> {
    let (address, prefix) = raw.trim().split_once('/')?;
    let address: std::net::IpAddr = address.trim().parse().ok()?;
    let prefix: u8 = prefix.trim().parse().ok()?;
    match address {
        std::net::IpAddr::V4(address) if prefix <= 32 => {
            let mask = if prefix == 0 {
                0
            } else {
                u32::MAX << (32 - prefix)
            };
            let network = std::net::Ipv4Addr::from(u32::from(address) & mask);
            Some(format!("{network}/{prefix}"))
        }
        std::net::IpAddr::V6(address) if prefix <= 128 => {
            let mask = if prefix == 0 {
                0
            } else {
                u128::MAX << (128 - prefix)
            };
            let network = std::net::Ipv6Addr::from(u128::from(address) & mask);
            Some(format!("{network}/{prefix}"))
        }
        _ => None,
    }
}

fn parse_scope_review_response(response: &str) -> Option<Vec<ScopingReviewedTarget>> {
    serde_json::from_str(response).ok()
}

fn valid_review_rows(rows: &[ScopingReviewedTarget], allow_empty: bool) -> bool {
    if rows.is_empty() {
        return allow_empty;
    }
    let mut identities = BTreeSet::new();
    rows.iter().all(|row| {
        row.scope.trim().eq_ignore_ascii_case("in")
            && canonical_scoping_target(row)
                .map(|identity| identities.insert(identity))
                .unwrap_or(false)
    })
}

fn exact_review_row(row: &ScopingReviewedTarget) -> (String, String, String) {
    (
        row.value.trim().to_string(),
        row.target_type.trim().to_ascii_lowercase(),
        row.scope.trim().to_ascii_lowercase(),
    )
}

fn unchanged_nonempty_subset(
    presented: &[ScopingReviewedTarget],
    selected: &[ScopingReviewedTarget],
) -> bool {
    if !valid_review_rows(presented, false) || !valid_review_rows(selected, false) {
        return false;
    }
    let selected_len = selected.len();
    let presented = presented
        .iter()
        .map(exact_review_row)
        .collect::<BTreeSet<_>>();
    let selected = selected
        .iter()
        .map(exact_review_row)
        .collect::<BTreeSet<_>>();
    selected.len() == selected_len && selected.is_subset(&presented)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(value: &str) -> ScopingReviewedTarget {
        ScopingReviewedTarget {
            value: value.to_string(),
            target_type: "domain".to_string(),
            scope: "in".to_string(),
        }
    }

    #[test]
    fn active_recon_scope_review_accepts_only_unchanged_nonempty_subset() {
        let presented = vec![row("a.example"), row("b.example")];
        assert!(unchanged_nonempty_subset(&presented, &[row("b.example")]));
        assert!(!unchanged_nonempty_subset(&presented, &[]));
        assert!(!unchanged_nonempty_subset(&presented, &[row("c.example")]));
        assert!(!unchanged_nonempty_subset(&presented, &[row("A.example")]));
    }
}

//! Explicit target-scope authority for an exact retained-DB resume.
//!
//! A resume cannot turn `--auto-approve` into permission for newly discovered
//! targets. This packet instead binds the operator decision to one operation,
//! engagement root, complete presented candidate set, and unchanged non-empty
//! selected subset. The existing orchestrator and DB transaction remain the
//! only writers; this module only supplies the exact review response.

use std::{collections::BTreeSet, path::Path};

use anyhow::{Context, Result};
use golish_agent_kit::db_traits::ScopingReviewedTarget;
use serde::Deserialize;
use uuid::Uuid;

const PACKET_SCHEMA: &str = "stage_run_active_recon_scope_authority.v1";
const MAX_PACKET_BYTES: u64 = 64 * 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScopeAuthorityPacket {
    schema: String,
    operation_id: Uuid,
    organization_id: Uuid,
    expected_presented: Vec<ScopingReviewedTarget>,
    selected: Vec<ScopingReviewedTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RetainedScopeAuthority {
    expected_presented: BTreeSet<(String, String, String)>,
    selected: Vec<ScopingReviewedTarget>,
}

impl RetainedScopeAuthority {
    pub(super) fn resolve_response(&self, context: &str) -> Option<String> {
        let presented: Vec<ScopingReviewedTarget> = serde_json::from_str(context).ok()?;
        let presented = exact_rows(&presented)?;
        if presented != self.expected_presented {
            return None;
        }
        serde_json::to_string(&self.selected).ok()
    }
}

pub(super) fn read_retained_scope_authority(
    path: &Path,
    expected_operation_id: Uuid,
    expected_organization_id: Uuid,
) -> Result<RetainedScopeAuthority> {
    anyhow::ensure!(
        path.is_absolute(),
        "--stage-run-active-recon-scope-authority must be an absolute path"
    );
    let metadata = std::fs::symlink_metadata(path).with_context(|| {
        format!(
            "inspect active-recon scope authority packet {}",
            path.display()
        )
    })?;
    anyhow::ensure!(
        metadata.file_type().is_file() && !metadata.file_type().is_symlink(),
        "active-recon scope authority packet must be a regular non-symlink file"
    );
    anyhow::ensure!(
        metadata.len() <= MAX_PACKET_BYTES,
        "active-recon scope authority packet exceeds {MAX_PACKET_BYTES} bytes"
    );
    let bytes = std::fs::read(path).with_context(|| {
        format!(
            "read active-recon scope authority packet {}",
            path.display()
        )
    })?;
    let packet: ScopeAuthorityPacket =
        serde_json::from_slice(&bytes).context("parse active-recon scope authority packet")?;
    anyhow::ensure!(
        packet.schema == PACKET_SCHEMA,
        "unsupported active-recon scope authority packet schema"
    );
    anyhow::ensure!(
        packet.operation_id == expected_operation_id
            && packet.organization_id == expected_organization_id,
        "active-recon scope authority packet does not match the exact resume"
    );

    let expected_presented = exact_rows(&packet.expected_presented)
        .context("active-recon scope authority presented set is invalid")?;
    let selected = exact_rows(&packet.selected)
        .context("active-recon scope authority selected set is invalid")?;
    anyhow::ensure!(
        !expected_presented.is_empty()
            && !selected.is_empty()
            && selected.is_subset(&expected_presented),
        "active-recon scope authority must select an unchanged non-empty subset"
    );

    Ok(RetainedScopeAuthority {
        expected_presented,
        selected: packet.selected,
    })
}

fn exact_rows(rows: &[ScopingReviewedTarget]) -> Option<BTreeSet<(String, String, String)>> {
    if rows.is_empty() {
        return None;
    }
    let exact = rows
        .iter()
        .map(|row| {
            let value = row.value.trim().to_string();
            let target_type = row.target_type.trim().to_ascii_lowercase();
            let scope = row.scope.trim().to_ascii_lowercase();
            (!value.is_empty()
                && matches!(
                    target_type.as_str(),
                    "domain" | "ip" | "cidr" | "url" | "wildcard"
                )
                && scope == "in")
                .then_some((value, target_type, scope))
        })
        .collect::<Option<BTreeSet<_>>>()?;
    (exact.len() == rows.len()).then_some(exact)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(value: &str, target_type: &str) -> ScopingReviewedTarget {
        ScopingReviewedTarget {
            value: value.to_owned(),
            target_type: target_type.to_owned(),
            scope: "in".to_owned(),
        }
    }

    #[test]
    fn retained_scope_authority_resolves_only_the_exact_presented_set() {
        let authority = RetainedScopeAuthority {
            expected_presented: exact_rows(&[
                row("https://moresec.cn", "url"),
                row("moresec.cn", "domain"),
            ])
            .expect("exact presented"),
            selected: vec![row("moresec.cn", "domain")],
        };
        let exact = serde_json::to_string(&vec![
            row("moresec.cn", "domain"),
            row("https://moresec.cn", "url"),
        ])
        .expect("serialize exact context");
        assert_eq!(
            serde_json::from_str::<Vec<ScopingReviewedTarget>>(
                &authority.resolve_response(&exact).expect("exact response")
            )
            .expect("parse response"),
            vec![row("moresec.cn", "domain")]
        );

        let drifted = serde_json::to_string(&vec![row("other.example", "domain")])
            .expect("serialize drifted context");
        assert!(authority.resolve_response(&drifted).is_none());
    }
}

use golish_agent_kit::db_traits::StageWorkItemView;
use golish_sub_agents::InvestigationAssetLaneIdentity;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InvestigationAssetPrimaryIdentity {
    pub lane: InvestigationAssetLaneIdentity,
    pub evolution_epoch: i64,
    pub schedule_round: Option<i64>,
}

/// Recover the exact current asset identity from the server-created Primary.
/// Historical fixed-roster WorkItems are intentionally not part of this
/// identity or the current Analysis denominator.
pub(crate) fn investigation_asset_primary_identity(
    item: &StageWorkItemView,
) -> Result<InvestigationAssetPrimaryIdentity, &'static str> {
    let Some([marker]) = item.input_refs.as_array().map(Vec::as_slice) else {
        return Err("INVESTIGATION_ASSET_PRIMARY_MARKER_INVALID");
    };
    let Some(marker) = marker
        .as_object()
        .filter(|marker| matches!(marker.len(), 5 | 6))
    else {
        return Err("INVESTIGATION_ASSET_PRIMARY_MARKER_INVALID");
    };
    if marker.get("kind").and_then(Value::as_str) != Some("investigation_asset_lane") {
        return Err("INVESTIGATION_ASSET_PRIMARY_MARKER_INVALID");
    }
    let asset_lane_id = marker
        .get("asset_lane_id")
        .and_then(Value::as_str)
        .and_then(|value| uuid::Uuid::parse_str(value).ok())
        .filter(|value| !value.is_nil())
        .ok_or("INVESTIGATION_ASSET_PRIMARY_MARKER_INVALID")?;
    let target_id = marker
        .get("target_id")
        .and_then(Value::as_str)
        .and_then(|value| uuid::Uuid::parse_str(value).ok())
        .filter(|value| !value.is_nil())
        .ok_or("INVESTIGATION_ASSET_PRIMARY_MARKER_INVALID")?;
    let asset_context_sha256 = marker
        .get("asset_context_sha256")
        .and_then(Value::as_str)
        .ok_or("INVESTIGATION_ASSET_PRIMARY_MARKER_INVALID")?
        .to_string();
    let evolution_epoch = marker
        .get("evolution_epoch")
        .and_then(Value::as_i64)
        .filter(|value| *value >= 0)
        .ok_or("INVESTIGATION_ASSET_PRIMARY_MARKER_INVALID")?;
    let schedule_round = marker
        .get("schedule_round")
        .map(|value| {
            value
                .as_i64()
                .filter(|value| *value >= 0)
                .ok_or("INVESTIGATION_ASSET_PRIMARY_MARKER_INVALID")
        })
        .transpose()?;
    if marker.len() != if schedule_round.is_some() { 6 } else { 5 } {
        return Err("INVESTIGATION_ASSET_PRIMARY_MARKER_INVALID");
    }
    let lane = InvestigationAssetLaneIdentity {
        asset_lane_id,
        target_id,
        asset_context_sha256,
    };
    lane.validate()?;
    let expected_id = uuid::Uuid::new_v5(
        &asset_lane_id,
        match schedule_round {
            Some(schedule_round) => {
                format!(
                    "investigation-asset-primary-work-item-v2:{evolution_epoch}:{schedule_round}"
                )
            }
            None => format!("investigation-asset-primary-work-item-v1:{evolution_epoch}"),
        }
        .as_bytes(),
    );
    let expected_stable_key = match schedule_round {
        Some(schedule_round) => {
            format!("asset:{asset_lane_id}:primary:{evolution_epoch}:round:{schedule_round}")
        }
        None => format!("asset:{asset_lane_id}:primary:{evolution_epoch}"),
    };
    if item.id != expected_id
        || item.stable_key != expected_stable_key
        || item.work_item_kind != "investigation_asset_primary"
        || item.role != "investigation"
        || item.input_manifest_hash != lane.asset_context_sha256
        || item.required_for_barrier
        || !item.is_aggregator
        || item.conflict_key.is_some()
        || item.created_by != "server_phase_transition"
    {
        return Err("INVESTIGATION_ASSET_PRIMARY_IDENTITY_MISMATCH");
    }
    Ok(InvestigationAssetPrimaryIdentity {
        lane,
        evolution_epoch,
        schedule_round,
    })
}

#[cfg(test)]
mod tests {
    use golish_agent_kit::db_traits::{RuntimeStageWorkItemStatus, StageWorkItemView};
    use serde_json::json;

    use super::investigation_asset_primary_identity;

    fn primary() -> StageWorkItemView {
        let asset_lane_id = uuid::Uuid::from_u128(1);
        let target_id = uuid::Uuid::from_u128(2);
        let evolution_epoch = 3_i64;
        let hash = format!("sha256:{}", "a".repeat(64));
        StageWorkItemView {
            id: uuid::Uuid::new_v5(
                &asset_lane_id,
                format!("investigation-asset-primary-work-item-v1:{evolution_epoch}").as_bytes(),
            ),
            stage_team_plan_id: uuid::Uuid::from_u128(3),
            stage_run_unit_id: uuid::Uuid::from_u128(4),
            organization_id: uuid::Uuid::from_u128(5),
            stable_key: format!("asset:{asset_lane_id}:primary:{evolution_epoch}"),
            work_item_kind: "investigation_asset_primary".to_string(),
            role: "investigation".to_string(),
            input_refs: json!([{
                "kind": "investigation_asset_lane",
                "asset_lane_id": asset_lane_id,
                "target_id": target_id,
                "asset_context_sha256": hash,
                "evolution_epoch": evolution_epoch,
            }]),
            input_manifest_hash: hash,
            priority: 0,
            required_for_barrier: false,
            is_aggregator: true,
            conflict_key: None,
            attempt_policy: json!({"max_attempts": 3}),
            budget: json!({}),
            output_schema: "stage_unit_aggregate.v1".to_string(),
            created_by: "server_phase_transition".to_string(),
            status: RuntimeStageWorkItemStatus::Queued,
            row_version: 1,
        }
    }

    #[test]
    fn primary_identity_does_not_require_a_fixed_roster() {
        let primary = primary();
        let identity = investigation_asset_primary_identity(&primary).expect("exact Primary");
        assert_eq!(identity.lane.target_id, uuid::Uuid::from_u128(2));
        assert_eq!(identity.evolution_epoch, 3);
    }

    #[test]
    fn primary_identity_rejects_foreign_or_role_shaped_items() {
        let mut foreign = primary();
        foreign.role = "browser".to_string();
        assert_eq!(
            investigation_asset_primary_identity(&foreign),
            Err("INVESTIGATION_ASSET_PRIMARY_IDENTITY_MISMATCH")
        );
    }

    #[test]
    fn primary_identity_accepts_dynamic_schedule_round_v2() {
        let mut primary = primary();
        primary.input_refs[0]["schedule_round"] = json!(4);
        primary.id = uuid::Uuid::new_v5(
            &uuid::Uuid::from_u128(1),
            b"investigation-asset-primary-work-item-v2:3:4",
        );
        primary.stable_key = format!("asset:{}:primary:3:round:4", uuid::Uuid::from_u128(1));
        let identity = investigation_asset_primary_identity(&primary).expect("v2 Primary");
        assert_eq!(identity.schedule_round, Some(4));
    }
}

//! Explicit local-admin maintenance entrypoints.
//!
//! These commands never enter the LLM/runtime stack and never accept a caller
//! principal. The active retained local operator is resolved from the DB.

use anyhow::Context as _;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;

use super::Args;

async fn run_promotion_on_pool(args: &Args, pool: &sqlx::PgPool) -> anyhow::Result<()> {
    let target_joint_rank = args
        .plan_d_maintenance_target_rank
        .ok_or_else(|| anyhow::anyhow!("plan_d_maintenance_target_rank_required"))?;
    if !args.plan_d_maintenance_apply && args.plan_d_maintenance_plan_hash.is_some() {
        anyhow::bail!("plan_d_maintenance_plan_hash_only_valid_for_apply");
    }
    let expected_plan_hash = args.plan_d_maintenance_plan_hash.as_deref();
    if args.plan_d_maintenance_apply
        && expected_plan_hash.is_none_or(|value| {
            value.len() != 71
                || !value.starts_with("sha256:")
                || !value[7..]
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
    {
        anyhow::bail!("plan_d_maintenance_plan_hash_invalid");
    }
    let principal = golish_db::repo::operator_principals::current_local(pool).await?;
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await?;
    let (campaign_held, admission_held, campaign_generation, admission_generation, hold_version): (
        bool,
        bool,
        i64,
        i64,
        i64,
    ) = sqlx::query_as(
        r#"SELECT campaign_dispatch_held,operation_admission_held,
                  campaign_dispatch_generation,operation_admission_generation,row_version
             FROM verification_campaign_safety_holds
            WHERE singleton=TRUE FOR SHARE"#,
    )
    .fetch_one(&mut *tx)
    .await?;
    if !campaign_held || !admission_held {
        anyhow::bail!("OPERATION_PROMOTION_SAFETY_HOLD_REQUIRED");
    }
    let tool_truth_row_version: i64 = sqlx::query_scalar(
        "SELECT row_version FROM tool_truth_rollout WHERE singleton=TRUE FOR SHARE",
    )
    .fetch_one(&mut *tx)
    .await?;
    let investigation_row_version: i64 = sqlx::query_scalar(
        "SELECT row_version FROM investigation_rollout WHERE singleton=TRUE FOR SHARE",
    )
    .fetch_one(&mut *tx)
    .await?;
    let receipt = golish_db::repo::operation_default_rollout::promote_operation_defaults(
        &mut tx,
        golish_db::repo::operation_default_rollout::PromoteOperationDefaults {
            expected_safety_hold_row_version: hold_version,
            expected_campaign_dispatch_generation: campaign_generation,
            expected_operation_admission_generation: admission_generation,
            expected_tool_truth_row_version: tool_truth_row_version,
            expected_investigation_row_version: investigation_row_version,
            target_joint_rank,
            expected_evidence_manifest_hash: expected_plan_hash.map(str::to_owned),
            principal_id: principal.id,
            reason: args
                .plan_d_maintenance_reason
                .clone()
                .unwrap_or_else(|| "local-admin Plan D rollout maintenance".to_owned()),
        },
    )
    .await
    .map_err(|error| anyhow::anyhow!(error.code()))?;
    let output = json!({
        "schema":"plan_d_local_admin_promotion.v1",
        "mode":if args.plan_d_maintenance_apply { "applied" } else { "dry_run" },
        "fromJointRank":receipt.from_joint_rank,
        "toJointRank":receipt.to_joint_rank,
        "evidenceManifestHash":receipt.evidence_manifest_hash,
        "evidenceMemberCount":receipt.evidence_member_count,
        "receiptId":if args.plan_d_maintenance_apply {
            Some(receipt.receipt_id)
        } else {
            None
        },
        "safetyHold":{
            "campaignDispatchHeld":campaign_held,
            "operationAdmissionHeld":admission_held,
            "rowVersion":hold_version,
            "campaignDispatchGeneration":campaign_generation,
            "operationAdmissionGeneration":admission_generation,
        },
        "cas":{
            "toolTruthRowVersion":tool_truth_row_version,
            "investigationRowVersion":investigation_row_version,
        },
    });
    if args.plan_d_maintenance_apply {
        tx.commit().await?;
    } else {
        tx.rollback().await?;
    }
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

pub async fn run_plan_d_maintenance(args: Args) -> anyhow::Result<()> {
    let config = golish_db::DbConfig::default();
    let preexisting = std::net::TcpStream::connect(("127.0.0.1", config.port)).is_ok();
    if preexisting {
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(&config.connection_string())
            .await
            .context("connect existing local Golish database")?;
        let result = run_promotion_on_pool(&args, &pool).await;
        pool.close().await;
        result
    } else {
        let mut db = golish_db::GolishDb::start(config)
            .await
            .context("start local Golish database")?;
        let result = run_promotion_on_pool(&args, db.pool()).await;
        db.stop().await;
        result
    }
}

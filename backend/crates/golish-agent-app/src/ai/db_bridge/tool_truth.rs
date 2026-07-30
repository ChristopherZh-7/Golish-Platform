use uuid::Uuid;

use golish_agent_kit::db_traits::{
    RecordToolTruthShadowAssessment, SealToolTruthDenominatorRequest,
    ToolTruthDenominatorSourceRef, ToolTruthDenominatorView,
};
use golish_agent_kit::harness::tool_truth::{
    build_denominator_items, evaluate_shadow_tool_truth, DenominatorAsset, ToolTruthReceiptCoverage,
};
use golish_agent_kit::harness::StageKind;

use super::GolishDbRepoProvider;

pub(super) fn stable_denominator_seal_request(stage_execution_id: Uuid, source_id: Uuid) -> Uuid {
    Uuid::new_v5(&stage_execution_id, source_id.as_bytes())
}

impl GolishDbRepoProvider {
    pub(super) async fn tool_truth_seal_denominator_impl(
        &self,
        request: SealToolTruthDenominatorRequest,
    ) -> anyhow::Result<ToolTruthDenominatorView> {
        let source = match request.source {
            ToolTruthDenominatorSourceRef::StageAssetWave {
                stage_asset_wave_id,
            } => {
                golish_db::repo::capability_execution_receipts::DenominatorSourceRef::StageAssetWave(
                    stage_asset_wave_id,
                )
            }
            ToolTruthDenominatorSourceRef::StageTeamUnit { stage_run_unit_id } => {
                golish_db::repo::capability_execution_receipts::DenominatorSourceRef::StageTeamUnit(
                    stage_run_unit_id,
                )
            }
        };
        let row = golish_db::repo::capability_execution_receipts::seal_source_denominator(
            &self.pool,
            &golish_db::repo::capability_execution_receipts::SealSourceDenominator {
                stable_seal_request_id: request.stable_seal_request_id,
                stage_execution_id: request.stage_execution_id,
                source,
            },
            |stage, locked| {
                let stage = StageKind::try_parse(stage)
                    .ok_or_else(|| anyhow::anyhow!("TOOL_TRUTH_STAGE_KIND_INVALID"))?;
                let assets = locked
                    .iter()
                    .map(|asset| DenominatorAsset {
                        target_id: asset.target_id,
                        exact_asset: asset.exact_asset.clone(),
                        asset_type: asset.asset_type.clone(),
                        web_capable: asset.web_capable,
                    })
                    .collect::<Vec<_>>();
                build_denominator_items(stage, &assets)
                    .map(|items| {
                        items
                            .into_iter()
                            .map(|item| {
                                golish_db::repo::capability_execution_receipts::CompiledDenominatorItem {
                                    input_key: item.input_key,
                                    target_id: item.target_id,
                                    exact_asset: item.exact_asset,
                                    technique: item.technique,
                                    expected_capability: item.expected_capability,
                                }
                            })
                            .collect()
                    })
                    .map_err(Into::into)
            },
        )
        .await?;
        Ok(ToolTruthDenominatorView {
            id: row.id,
            execution_authority_id: row.execution_authority_id,
            input_manifest_hash: row.input_manifest_hash,
            member_count: row
                .member_count
                .ok_or_else(|| anyhow::anyhow!("TOOL_TRUTH_DENOMINATOR_UNSEALED"))?,
            denominator_hash: row.denominator_hash,
        })
    }

    pub(super) async fn seal_wave_before_dispatch(
        &self,
        stage_execution_id: Uuid,
        operation_id: Uuid,
        stage_asset_wave_id: Uuid,
    ) -> anyhow::Result<()> {
        let contract =
            golish_db::repo::operation_state::get_tool_truth_contract(&self.pool, operation_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("TOOL_TRUTH_OPERATION_CONTRACT_MISSING"))?;
        if !contract.writes_receipts() {
            return Ok(());
        }
        if stage_execution_id.is_nil() {
            anyhow::bail!("TOOL_TRUTH_STAGE_EXECUTION_MISSING");
        }
        let denominator = self
            .tool_truth_seal_denominator_impl(SealToolTruthDenominatorRequest {
                stable_seal_request_id: stable_denominator_seal_request(
                    stage_execution_id,
                    stage_asset_wave_id,
                ),
                stage_execution_id,
                source: ToolTruthDenominatorSourceRef::StageAssetWave {
                    stage_asset_wave_id,
                },
            })
            .await?;
        tracing::debug!(
            denominator_id = %denominator.id,
            wave_id = %stage_asset_wave_id,
            "tool-truth denominator sealed before provider dispatch"
        );
        Ok(())
    }

    pub(super) async fn tool_truth_record_shadow_assessment_impl(
        &self,
        request: RecordToolTruthShadowAssessment,
    ) -> anyhow::Result<()> {
        let stage_execution_ids = sqlx::query_scalar::<_, Uuid>(
            r#"SELECT id FROM stage_runs
                WHERE operation_id=$1 AND stage_kind=$2 AND status='started'
                ORDER BY started_at,id FOR SHARE"#,
        )
        .bind(request.operation_id)
        .bind(&request.stage_kind)
        .fetch_all(&*self.pool)
        .await?;
        let [stage_execution_id] = stage_execution_ids.as_slice() else {
            anyhow::bail!("TOOL_TRUTH_ACTIVE_STAGE_EXECUTION_AMBIGUOUS");
        };

        let denominator = sqlx::query_as::<_, (Uuid, String, Uuid, String, Uuid, String, Uuid)>(
            r#"SELECT d.id,d.denominator_hash,a.id,a.authority_hash,
                      a.project_scope_id,a.project_path_at_freeze,a.scope_snapshot_id
                 FROM coverage_denominators d
                 JOIN tool_truth_execution_authorities a ON a.id=d.execution_authority_id
                WHERE d.operation_id=$1 AND d.organization_id=$2
                  AND d.stage_execution_id=$3 AND d.stage_kind=$4
                  AND d.denominator_kind='root' AND d.sealed_at IS NOT NULL
                  AND ($5::uuid IS NULL OR EXISTS (
                      SELECT 1 FROM tool_truth_stage_wave_execution_bindings b
                       WHERE b.id=a.stage_wave_binding_id AND b.stage_asset_wave_id=$5
                  ))
                ORDER BY d.created_at DESC,d.id DESC LIMIT 1"#,
        )
        .bind(request.operation_id)
        .bind(request.organization_id)
        .bind(stage_execution_id)
        .bind(&request.stage_kind)
        .bind(request.stage_asset_wave_id)
        .fetch_optional(&*self.pool)
        .await?;

        let (authority_id, authority_hash, project_scope_id, project_path, snapshot_id) =
            if let Some(denominator) = &denominator {
                (
                    denominator.2,
                    denominator.3.clone(),
                    denominator.4,
                    denominator.5.clone(),
                    denominator.6,
                )
            } else {
                let scope = sqlx::query_as::<_, (Uuid, String, Uuid)>(
                    r#"SELECT project_scope_id,project_path_at_freeze,id
                         FROM operation_org_scope_snapshots s
                        WHERE s.operation_id=$1 AND s.sealed_at IS NOT NULL
                          AND EXISTS (
                              SELECT 1 FROM operation_org_scope_units u
                               WHERE u.snapshot_id=s.id AND u.organization_id=$2
                          ) FOR SHARE"#,
                )
                .bind(request.operation_id)
                .bind(request.organization_id)
                .fetch_one(&*self.pool)
                .await?;
                let stable = Uuid::new_v5(
                    stage_execution_id,
                    format!(
                        "missing-denominator:{}:{}",
                        request.organization_id, request.stage_kind
                    )
                    .as_bytes(),
                );
                sqlx::query(
                    r#"INSERT INTO tool_truth_execution_authorities(
                           id,stable_authority_request_id,operation_id,project_scope_id,
                           project_path_at_freeze,scope_snapshot_id,organization_id,
                           stage_execution_id,stage_kind,execution_source_kind,
                           execution_owner_kind,authority_hash
                       ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'stage_execution','host_stage',$10)
                       ON CONFLICT(operation_id,stable_authority_request_id) DO NOTHING"#,
                )
                .bind(Uuid::new_v4())
                .bind(stable)
                .bind(request.operation_id)
                .bind(scope.0)
                .bind(&scope.1)
                .bind(scope.2)
                .bind(request.organization_id)
                .bind(stage_execution_id)
                .bind(&request.stage_kind)
                .bind(format!("sha256:{}", "0".repeat(64)))
                .execute(&*self.pool)
                .await?;
                let authority = sqlx::query_as::<_, (Uuid, String)>(
                    r#"SELECT id,authority_hash FROM tool_truth_execution_authorities
                        WHERE operation_id=$1 AND stable_authority_request_id=$2"#,
                )
                .bind(request.operation_id)
                .bind(stable)
                .fetch_one(&*self.pool)
                .await?;
                (authority.0, authority.1, scope.0, scope.1, scope.2)
            };

        let coverage = if let Some((denominator_id, _, _, _, _, _, _)) = denominator.as_ref() {
            let counts = sqlx::query_as::<_, (i64, i64, i64)>(
                r#"SELECT count(*)::bigint,
                          count(*) FILTER (WHERE EXISTS (
                              SELECT 1 FROM capability_execution_receipt_inputs i
                              JOIN capability_execution_receipts r ON r.id=i.receipt_id
                               WHERE i.denominator_item_id=di.id
                                 AND i.denominator_id=di.denominator_id
                                 AND i.sealed_at IS NOT NULL
                                 AND i.coverage_extent='complete'
                                 AND i.landing_state='committed'
                                 AND r.finalized_at IS NOT NULL
                                 AND r.reconciliation_state='consistent'
                                 AND r.current_semantic_reconciliation_id IS NOT NULL
                          ))::bigint,
                          count(*) FILTER (WHERE EXISTS (
                              SELECT 1 FROM capability_execution_receipt_inputs i
                               WHERE i.denominator_item_id=di.id
                                 AND i.denominator_id=di.denominator_id
                                 AND i.sealed_at IS NOT NULL
                                 AND i.coverage_extent IN ('partial','sampled','template_only')
                          ))::bigint
                     FROM coverage_denominator_items di WHERE di.denominator_id=$1"#,
            )
            .bind(denominator_id)
            .fetch_one(&*self.pool)
            .await?;
            Some(ToolTruthReceiptCoverage {
                expected: usize::try_from(counts.0)?,
                terminal: usize::try_from(counts.1)?,
                degraded: usize::try_from(counts.2)?,
            })
        } else {
            None
        };
        let assessment = evaluate_shadow_tool_truth(request.legacy_allowed, coverage, &[]);

        let authority_set_id = if let Some((denominator_id, denominator_hash, ..)) = &denominator {
            let stable = Uuid::new_v5(
                stage_execution_id,
                format!(
                    "authority-set:{denominator_hash}:{}:{}",
                    coverage.map_or(0, |value| value.terminal),
                    coverage.map_or(0, |value| value.degraded)
                )
                .as_bytes(),
            );
            let id = Uuid::new_v4();
            sqlx::query(
                r#"INSERT INTO tool_truth_authority_set_seals(
                       id,stable_consumer_request_id,execution_authority_id,denominator_id,
                       denominator_hash,consumer_kind,graph_hash,semantic_hash,freshness_hash
                   ) VALUES($1,$2,$3,$4,$5,'org_gate_shadow',
                       tool_truth_sha256($6),tool_truth_sha256($7),tool_truth_sha256($8))
                   ON CONFLICT(execution_authority_id,stable_consumer_request_id) DO NOTHING"#,
            )
            .bind(id)
            .bind(stable)
            .bind(authority_id)
            .bind(denominator_id)
            .bind(denominator_hash)
            .bind(format!("graph:{denominator_hash}"))
            .bind(format!("semantic:{denominator_hash}"))
            .bind(format!("freshness:{denominator_hash}"))
            .execute(&*self.pool)
            .await?;
            let id = sqlx::query_scalar::<_, Uuid>(
                r#"SELECT id FROM tool_truth_authority_set_seals
                    WHERE execution_authority_id=$1 AND stable_consumer_request_id=$2"#,
            )
            .bind(authority_id)
            .bind(stable)
            .fetch_one(&*self.pool)
            .await?;
            let sealed: bool = sqlx::query_scalar(
                "SELECT sealed_at IS NOT NULL FROM tool_truth_authority_set_seals WHERE id=$1",
            )
            .bind(id)
            .fetch_one(&*self.pool)
            .await?;
            if !sealed {
                sqlx::query(
                    "UPDATE tool_truth_authority_set_seals SET sealed_at=statement_timestamp() WHERE id=$1",
                )
                .bind(id)
                .execute(&*self.pool)
                .await?;
            }
            Some(id)
        } else {
            None
        };

        let basis = if denominator.is_some() {
            "authority_set"
        } else {
            "missing_denominator"
        };
        let expected = coverage.map_or(0_i64, |value| value.expected as i64);
        let terminal = coverage.map_or(0_i64, |value| value.terminal as i64);
        let degraded = coverage.map_or(0_i64, |value| value.degraded as i64);
        let stable_gate_request_id = Uuid::new_v5(
            stage_execution_id,
            format!(
                "gate:{basis}:{}:{expected}:{terminal}:{degraded}:{}",
                request.organization_id, request.legacy_allowed
            )
            .as_bytes(),
        );
        let denominator_id = denominator.as_ref().map(|value| value.0);
        let residual = if denominator_id.is_none() {
            serde_json::json!({"reason_code": "TOOL_TRUTH_DENOMINATOR_MISSING"})
        } else {
            serde_json::json!({"missing_item_count": expected-terminal})
        };
        sqlx::query(
            r#"INSERT INTO tool_truth_gate_assessments(
                   id,stable_gate_request_id,operation_id,project_scope_id,
                   project_path_at_freeze,scope_snapshot_id,organization_id,
                   stage_execution_id,stage_kind,execution_authority_id,
                   execution_authority_hash,assessment_basis_kind,denominator_id,
                   authority_set_id,legacy_allowed,control_decision,coverage_grade,
                   divergence,expected_item_count,terminal_item_count,degraded_item_count,
                   residual,assessment_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,
                        $18,$19,$20,$21,$22,
                        tool_truth_sha256(jsonb_build_object(
                            'stable_gate_request_id',$2,'execution_authority_id',$10,
                            'basis',$12,'denominator_id',$13,'authority_set_id',$14,
                            'legacy_allowed',$15,'control_decision',$16,'coverage_grade',$17,
                            'expected',$19,'terminal',$20,'degraded',$21,'residual',$22
                        )::text))
               ON CONFLICT(operation_id,stable_gate_request_id) DO NOTHING"#,
        )
        .bind(Uuid::new_v4())
        .bind(stable_gate_request_id)
        .bind(request.operation_id)
        .bind(project_scope_id)
        .bind(&project_path)
        .bind(snapshot_id)
        .bind(request.organization_id)
        .bind(stage_execution_id)
        .bind(&request.stage_kind)
        .bind(authority_id)
        .bind(&authority_hash)
        .bind(basis)
        .bind(denominator_id)
        .bind(authority_set_id)
        .bind(request.legacy_allowed)
        .bind(assessment.control_decision.as_str())
        .bind(assessment.coverage_grade.as_str())
        .bind(assessment.divergence)
        .bind(expected)
        .bind(terminal)
        .bind(degraded)
        .bind(residual)
        .execute(&*self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reserve_local_port() -> u16 {
        std::net::TcpListener::bind(("127.0.0.1", 0))
            .expect("reserve local postgres port")
            .local_addr()
            .expect("read reserved port")
            .port()
    }

    #[test]
    fn public_denominator_request_cannot_omit_members_or_rebind_a_source() {
        let stage_execution_id = Uuid::new_v4();
        let source_id = Uuid::new_v4();
        let request = SealToolTruthDenominatorRequest {
            stable_seal_request_id: stable_denominator_seal_request(stage_execution_id, source_id),
            stage_execution_id,
            source: ToolTruthDenominatorSourceRef::StageAssetWave {
                stage_asset_wave_id: source_id,
            },
        };
        assert_eq!(
            request.stable_seal_request_id,
            stable_denominator_seal_request(stage_execution_id, source_id)
        );
        assert_ne!(
            request.stable_seal_request_id,
            stable_denominator_seal_request(stage_execution_id, Uuid::new_v4())
        );
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn tool_truth_shadow_write_is_operation_and_org_scoped() {
        let data_dir = tempfile::tempdir().expect("temporary postgres directory");
        let mut db = golish_db::GolishDb::start(golish_db::DbConfig {
            pg_data_dir: data_dir.path().join("pgdata"),
            port: reserve_local_port(),
            database: format!("tool_truth_shadow_{}", Uuid::new_v4().simple()),
            ..golish_db::DbConfig::default()
        })
        .await
        .expect("start isolated migrated postgres");
        let pool = db.pool();
        let session_id = Uuid::new_v4();
        let operation_id = Uuid::new_v4();
        let project_scope_id = Uuid::new_v4();
        let stage_execution_id = Uuid::new_v4();
        let organization_id = Uuid::new_v4();
        let outside_organization_id = Uuid::new_v4();
        let scope_decision_id = Uuid::new_v4();
        let scope_snapshot_id = Uuid::new_v4();
        let project_path = format!("/tmp/tool-truth-shadow-{}", Uuid::new_v4().simple());
        sqlx::query(
            "INSERT INTO sessions(id,title,status,project_path) VALUES($1,'shadow','running',$2)",
        )
        .bind(session_id)
        .bind(&project_path)
        .execute(pool)
        .await
        .expect("insert session");
        sqlx::query("INSERT INTO tasks(id,session_id,title,input,status) VALUES($1,$2,'shadow','fixture','running')")
            .bind(operation_id)
            .bind(session_id)
            .execute(pool)
            .await
            .expect("insert operation task");
        sqlx::query("INSERT INTO project_scopes(project_scope_id,canonical_project_path,path_sha256) VALUES($1,$2,$3)")
            .bind(project_scope_id)
            .bind(&project_path)
            .bind(format!("sha256:{}", "1".repeat(64)))
            .execute(pool)
            .await
            .expect("insert project scope");
        sqlx::query("INSERT INTO operation_state(operation_id,profile,current_stage,runtime_memory_contract,project_scope_id) VALUES($1,'assessment','enumeration','legacy_v1',$2)")
            .bind(operation_id)
            .bind(project_scope_id)
            .execute(pool)
            .await
            .expect("insert operation state");
        sqlx::query("ALTER TABLE operation_state DISABLE TRIGGER operation_state_tool_truth_contract_immutable")
            .execute(pool)
            .await
            .expect("enable isolated shadow fixture");
        sqlx::query(
            "UPDATE operation_state SET tool_truth_contract='shadow_v1' WHERE operation_id=$1",
        )
        .bind(operation_id)
        .execute(pool)
        .await
        .expect("freeze shadow contract in isolated fixture");
        sqlx::query("ALTER TABLE operation_state ENABLE TRIGGER operation_state_tool_truth_contract_immutable")
            .execute(pool)
            .await
            .expect("restore contract guard");
        sqlx::query("INSERT INTO organizations(id,project_path,name) VALUES($1,$3,'Scoped'),($2,$3,'Outside')")
            .bind(organization_id)
            .bind(outside_organization_id)
            .bind(&project_path)
            .execute(pool)
            .await
            .expect("insert organizations");
        sqlx::query("INSERT INTO stage_runs(id,operation_id,stage_kind,status) VALUES($1,$2,'enumeration','started')")
            .bind(stage_execution_id)
            .bind(operation_id)
            .execute(pool)
            .await
            .expect("insert active stage execution");
        sqlx::query(
            r#"INSERT INTO operation_scope_decisions(
                   id,operation_id,project_scope_id,stage_execution_id,
                   root_organization_id,mode,decision_rows,decision_hash
               ) VALUES($1,$2,$3,$4,$5,'cli_flags','[]',$6)"#,
        )
        .bind(scope_decision_id)
        .bind(operation_id)
        .bind(project_scope_id)
        .bind(stage_execution_id)
        .bind(organization_id)
        .bind(format!("sha256:{}", "2".repeat(64)))
        .execute(pool)
        .await
        .expect("insert scope decision");
        let mut tx = pool.begin().await.expect("begin scope freeze");
        sqlx::query(
            r#"INSERT INTO operation_org_scope_snapshots(
                   id,operation_id,project_scope_id,scope_decision_id,
                   project_path_at_freeze,root_organization_id,mode,scope_hash
               ) VALUES($1,$2,$3,$4,$5,$6,'cli_flags',$7)"#,
        )
        .bind(scope_snapshot_id)
        .bind(operation_id)
        .bind(project_scope_id)
        .bind(scope_decision_id)
        .bind(&project_path)
        .bind(organization_id)
        .bind(format!("sha256:{}", "3".repeat(64)))
        .execute(&mut *tx)
        .await
        .expect("insert scope snapshot");
        sqlx::query(
            r#"INSERT INTO operation_org_scope_units(
                   snapshot_id,organization_id,organization_name_at_freeze,
                   role,depth,ordinal,decision_row_id,approval_source
               ) VALUES($1,$2,'Scoped','root',0,0,'root','{}')"#,
        )
        .bind(scope_snapshot_id)
        .bind(organization_id)
        .execute(&mut *tx)
        .await
        .expect("insert scoped organization");
        sqlx::query("UPDATE operation_org_scope_snapshots SET sealed_at=NOW() WHERE id=$1")
            .bind(scope_snapshot_id)
            .execute(&mut *tx)
            .await
            .expect("seal scope snapshot");
        tx.commit().await.expect("commit scope freeze");

        let provider = GolishDbRepoProvider::new(std::sync::Arc::new(pool.clone()));
        provider
            .tool_truth_record_shadow_assessment_impl(RecordToolTruthShadowAssessment {
                operation_id,
                organization_id,
                stage_kind: "enumeration".to_string(),
                stage_asset_wave_id: None,
                legacy_allowed: true,
            })
            .await
            .expect("record missing-denominator shadow assessment");
        let assessment: (String, String, bool) = sqlx::query_as(
            "SELECT control_decision,coverage_grade,divergence FROM tool_truth_gate_assessments WHERE operation_id=$1",
        )
        .bind(operation_id)
        .fetch_one(pool)
        .await
        .expect("read persisted shadow assessment");
        assert_eq!(
            assessment,
            ("hold".to_string(), "incomplete".to_string(), true)
        );

        let error = provider
            .tool_truth_record_shadow_assessment_impl(RecordToolTruthShadowAssessment {
                operation_id,
                organization_id: outside_organization_id,
                stage_kind: "enumeration".to_string(),
                stage_asset_wave_id: None,
                legacy_allowed: false,
            })
            .await
            .expect_err("outside organization cannot receive a scoped assessment");
        assert!(error.to_string().contains("no rows returned"));
        drop(provider);
        db.stop().await;
    }
}

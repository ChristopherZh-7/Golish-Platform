use std::path::{Path, PathBuf};

use golish_app_core::{DbState, GolishError, TauriEventEmitter};
use uuid::Uuid;

use crate::asset_intel::ToolsConfigState;
use crate::targets::{db_target_list, Target};

use super::export::write_recon_assets_workbook_file;
use super::runner::normalized_current_asset_records;
use super::runner::{initial_snapshot, OrganizationReconRunner};
use super::state::OrganizationReconState;
use super::types::{
    OrganizationReconExportResult, OrganizationReconRunSnapshot, OrganizationReconStageName,
    OrganizationReconStartArgs, ReconArtifactRef, ReconTaskStatus,
};

#[tauri::command]
pub async fn organization_recon_start_run(
    app: tauri::AppHandle,
    db: tauri::State<'_, DbState>,
    tools: tauri::State<'_, ToolsConfigState>,
    state: tauri::State<'_, OrganizationReconState>,
    args: OrganizationReconStartArgs,
) -> Result<OrganizationReconRunSnapshot, GolishError> {
    let pool = db.pool_ready().await?;
    let organization_id: Uuid = args.organization_id.parse()?;
    let organization = golish_db::repo::organizations::get_one(pool, organization_id)
        .await?
        .ok_or_else(|| GolishError::NotFound(format!("organization {}", args.organization_id)))?;
    if organization.name.trim().is_empty() {
        return Err(GolishError::Validation(
            "organization name is empty; cannot run recon".into(),
        ));
    }
    let run_id = Uuid::new_v4().to_string();
    let snapshot = initial_snapshot(
        run_id.clone(),
        args.organization_id.clone(),
        organization.project_path.clone(),
    );
    state.insert(snapshot.clone()).await;
    let snapshot = state.start_run(&run_id).await.unwrap_or(snapshot);

    let runner = OrganizationReconRunner {
        pool: pool.clone(),
        tools: tools.inner().clone(),
        state: state.inner().clone(),
        sink: Some(TauriEventEmitter::handle(app)),
    };
    tokio::spawn(runner.run(run_id, args, organization));
    Ok(snapshot)
}

#[tauri::command]
pub async fn organization_recon_get_run(
    state: tauri::State<'_, OrganizationReconState>,
    run_id: String,
) -> Result<OrganizationReconRunSnapshot, GolishError> {
    state
        .get(&run_id)
        .await
        .ok_or_else(|| GolishError::NotFound(format!("organization recon run {run_id}")))
}

#[tauri::command]
pub async fn organization_recon_export_assets(
    state: tauri::State<'_, OrganizationReconState>,
    run_id: String,
    output_path: String,
) -> Result<OrganizationReconExportResult, GolishError> {
    let _ = Uuid::parse_str(&run_id)?;
    let snapshot = state
        .get(&run_id)
        .await
        .ok_or_else(|| GolishError::NotFound(format!("organization recon run {run_id}")))?;
    let artifact = find_recon_assets_workbook(&snapshot)?;
    copy_recon_assets_workbook(artifact, Path::new(&output_path))
}

#[tauri::command]
pub async fn organization_recon_export_current_assets(
    db: tauri::State<'_, DbState>,
    organization_id: String,
    output_path: String,
) -> Result<OrganizationReconExportResult, GolishError> {
    let pool = db.pool_ready().await?;
    let organization_id: Uuid = organization_id.parse()?;
    let organization = golish_db::repo::organizations::get_one(pool, organization_id)
        .await?
        .ok_or_else(|| GolishError::NotFound(format!("organization {organization_id}")))?;
    let targets = organization_targets(pool, &organization.project_path, organization_id).await?;
    let records = normalized_current_asset_records(
        "current",
        &organization,
        &targets,
        "current/organization-profile.json",
    );
    let output_path = Path::new(&output_path);
    let bytes = write_recon_assets_workbook_file(output_path, &records)?;
    Ok(OrganizationReconExportResult {
        output_path: output_path.display().to_string(),
        bytes,
    })
}

async fn organization_targets(
    pool: &sqlx::PgPool,
    project_path: &str,
    organization_id: Uuid,
) -> Result<Vec<Target>, GolishError> {
    Ok(db_target_list(pool, Some(project_path))
        .await?
        .into_iter()
        .filter(|target| target.organization_id.as_deref() == Some(&organization_id.to_string()))
        .collect())
}

fn find_recon_assets_workbook(
    run: &OrganizationReconRunSnapshot,
) -> Result<&ReconArtifactRef, GolishError> {
    let task = run
        .tasks
        .iter()
        .find(|task| task.stage == OrganizationReconStageName::Processing)
        .ok_or_else(|| GolishError::NotFound("organization recon processing stage".into()))?;
    if !matches!(
        task.status,
        ReconTaskStatus::Completed | ReconTaskStatus::CheckedEmpty
    ) {
        return Err(GolishError::Validation(
            "recon asset workbook is only available after processing finishes".into(),
        ));
    }
    task.artifacts
        .iter()
        .find(|artifact| is_recon_assets_workbook(artifact))
        .ok_or_else(|| {
            GolishError::NotFound("processing artifact exports/recon-assets.xlsx".into())
        })
}

fn is_recon_assets_workbook(artifact: &ReconArtifactRef) -> bool {
    artifact.kind == "asset_workbook"
        && Path::new(&artifact.path)
            .file_name()
            .and_then(|name| name.to_str())
            == Some("recon-assets.xlsx")
}

fn copy_recon_assets_workbook(
    artifact: &ReconArtifactRef,
    output_path: &Path,
) -> Result<OrganizationReconExportResult, GolishError> {
    if output_path.as_os_str().is_empty() {
        return Err(GolishError::Validation("output path is empty".into()));
    }
    let source = PathBuf::from(&artifact.path);
    if !source.is_file() {
        return Err(GolishError::NotFound(format!(
            "recon asset workbook {}",
            source.display()
        )));
    }
    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let bytes = std::fs::copy(&source, output_path)?;
    Ok(OrganizationReconExportResult {
        output_path: output_path.display().to_string(),
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::organization_recon::runner::initial_snapshot;

    #[test]
    fn finds_processing_workbook_after_stage_four_finishes() {
        let mut run = initial_snapshot(
            Uuid::new_v4().to_string(),
            "org".into(),
            "/tmp/project".into(),
        );
        let processing = run
            .tasks
            .iter_mut()
            .find(|task| task.stage == OrganizationReconStageName::Processing)
            .unwrap();
        processing.status = ReconTaskStatus::Completed;
        processing.artifacts.push(ReconArtifactRef {
            path: "/tmp/run/processing/processing/exports/recon-assets.xlsx".into(),
            kind: "asset_workbook".into(),
            bytes: 42,
        });

        let artifact = find_recon_assets_workbook(&run).unwrap();

        assert_eq!(artifact.bytes, 42);
    }

    #[test]
    fn workbook_is_unavailable_before_processing_finishes() {
        let run = initial_snapshot(
            Uuid::new_v4().to_string(),
            "org".into(),
            "/tmp/project".into(),
        );

        let error = find_recon_assets_workbook(&run).unwrap_err();

        assert!(format!("{error}").contains("after processing finishes"));
    }

    #[test]
    fn export_copies_workbook_to_requested_path() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("recon-assets.xlsx");
        let output = dir.path().join("downloads").join("copy.xlsx");
        std::fs::write(&source, b"workbook").unwrap();
        let artifact = ReconArtifactRef {
            path: source.display().to_string(),
            kind: "asset_workbook".into(),
            bytes: 8,
        };

        let result = copy_recon_assets_workbook(&artifact, &output).unwrap();

        assert_eq!(result.bytes, 8);
        assert_eq!(std::fs::read(output).unwrap(), b"workbook");
    }
}

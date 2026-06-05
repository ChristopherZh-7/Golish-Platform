use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;

use golish_app_core::GolishError;

use super::types::{ReconArtifactRef, ReconTaskError, ReconTaskManifest};

fn checked_path(root: &Path, relative: &Path) -> Result<PathBuf, GolishError> {
    if relative.is_absolute()
        || relative.components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(GolishError::Validation(format!(
            "artifact path must stay below task dir: {}",
            relative.display()
        )));
    }
    Ok(root.join(relative))
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), GolishError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension(format!(
        "{}.tmp.{}",
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or("dat"),
        std::process::id()
    ));
    fs::write(&temp, bytes)?;
    fs::rename(temp, path)?;
    Ok(())
}

pub(crate) fn write_raw_bytes(
    task_dir: &Path,
    relative: impl AsRef<Path>,
    bytes: &[u8],
    kind: impl Into<String>,
) -> Result<ReconArtifactRef, GolishError> {
    let relative = relative.as_ref();
    let path = checked_path(task_dir, relative)?;
    atomic_write(&path, bytes)?;
    Ok(ReconArtifactRef {
        path: path.display().to_string(),
        kind: kind.into(),
        bytes: bytes.len() as u64,
    })
}

pub(crate) fn write_json_manifest(
    task_dir: &Path,
    manifest: &ReconTaskManifest,
) -> Result<PathBuf, GolishError> {
    let path = task_dir.join("manifest.json");
    let bytes = serde_json::to_vec_pretty(manifest).map_err(|error| {
        GolishError::Internal(format!("serialize recon manifest failed: {error}"))
    })?;
    atomic_write(&path, &bytes)?;
    Ok(path)
}

pub(crate) fn write_records_jsonl<T: Serialize>(
    task_dir: &Path,
    records: &[T],
) -> Result<ReconArtifactRef, GolishError> {
    let mut bytes = Vec::new();
    for record in records {
        serde_json::to_writer(&mut bytes, record).map_err(|error| {
            GolishError::Internal(format!("serialize normalized recon record failed: {error}"))
        })?;
        bytes.push(b'\n');
    }
    write_raw_bytes(
        task_dir,
        Path::new("normalized").join("records.jsonl"),
        &bytes,
        "normalized_records",
    )
}

pub(crate) fn decode_utf8_clean(bytes: &[u8]) -> Result<String, ReconTaskError> {
    let raw = std::str::from_utf8(bytes)
        .map_err(|error| ReconTaskError::new("decode_error", format!("invalid UTF-8: {error}")))?;
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    let without_ansi = golish_core::utils::strip_ansi(raw);
    let cleaned: String = without_ansi
        .chars()
        .filter(|character| matches!(*character, '\n' | '\r' | '\t') || !character.is_control())
        .collect();
    if cleaned.contains('\u{fffd}') {
        return Err(ReconTaskError::new(
            "decode_error",
            "decoded text contains replacement characters",
        ));
    }
    Ok(cleaned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::organization_recon::types::{ReconTaskManifest, ReconTaskStatus};

    #[test]
    fn decode_utf8_clean_strips_bom_ansi_and_controls() {
        let output = decode_utf8_clean(b"\xef\xbb\xbf\x1b[31mhello\x1b[0m\0\n").unwrap();

        assert_eq!(output, "hello\n");
    }

    #[test]
    fn decode_utf8_clean_rejects_invalid_bytes() {
        let error = decode_utf8_clean(&[0xff, 0xfe]).unwrap_err();

        assert_eq!(error.code, "decode_error");
    }

    #[test]
    fn raw_and_manifest_writes_stay_under_task_dir() {
        let dir = tempfile::tempdir().unwrap();
        let artifact = write_raw_bytes(dir.path(), "raw/stdout.log", b"ok\n", "stdout").unwrap();
        let mut manifest = ReconTaskManifest::new("run", "task", "stage", "source");
        manifest.status = ReconTaskStatus::Completed;
        manifest.artifacts.push(artifact);

        let manifest_path = write_json_manifest(dir.path(), &manifest).unwrap();

        assert!(manifest_path.exists());
        assert!(dir.path().join("raw/stdout.log").exists());
    }

    #[test]
    fn raw_write_rejects_parent_directory_escape() {
        let dir = tempfile::tempdir().unwrap();
        let error = write_raw_bytes(dir.path(), "../outside", b"bad", "raw").unwrap_err();

        assert!(format!("{error}").contains("must stay below task dir"));
    }
}

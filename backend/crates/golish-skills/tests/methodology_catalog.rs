use golish_core::methodology_context::{
    sha256_bytes, DeterministicDocumentId, MethodologyContractError,
    MethodologyDocumentDescriptorV1, MethodologyQueryV1, MethodologyTrustPolicyV1,
};
use golish_skills::{MethodologyCatalogError, MethodologyCatalogV1};
use serde_json::Value;
use std::path::{Path, PathBuf};

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/methodology-corpus")
}

fn policy() -> MethodologyTrustPolicyV1 {
    MethodologyTrustPolicyV1::new(1, ["Apache-2.0".to_string()]).unwrap()
}

fn load_fixture() -> MethodologyCatalogV1 {
    MethodologyCatalogV1::load(&fixture_root(), &policy()).unwrap()
}

fn copy_fixture() -> tempfile::TempDir {
    let temporary = tempfile::tempdir().unwrap();
    copy_directory(&fixture_root(), temporary.path());
    temporary
}

fn copy_directory(source: &Path, destination: &Path) {
    std::fs::create_dir_all(destination).unwrap();
    let mut entries = std::fs::read_dir(source)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let destination_path = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_directory(&entry.path(), &destination_path);
        } else {
            std::fs::copy(entry.path(), destination_path).unwrap();
        }
    }
}

fn mutate_manifest(root: &Path, mutate: impl FnOnce(&mut Value)) {
    let path = root.join("manifest.json");
    let mut value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    mutate(&mut value);
    std::fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}

#[test]
fn methodology_corpus_manifest_binds_revision_license_count_and_root_hash() {
    let catalog = load_fixture();
    assert_eq!(catalog.manifest().upstream_revision, "synthetic-fixture-v1");
    assert_eq!(catalog.manifest().license_spdx, "Apache-2.0");
    assert_eq!(catalog.manifest().document_count, 2);
    assert_eq!(catalog.documents().len(), 2);
    assert!(!catalog.manifest().instruction_authority());

    for field in [
        "upstream_revision",
        "license_text_sha256",
        "content_root_sha256",
    ] {
        let temporary = copy_fixture();
        mutate_manifest(temporary.path(), |manifest| {
            manifest[field] = Value::String(match field {
                "upstream_revision" => "tampered-revision".into(),
                _ => format!("sha256:{}", "0".repeat(64)),
            });
        });
        assert!(MethodologyCatalogV1::load(temporary.path(), &policy()).is_err());
    }

    let temporary = copy_fixture();
    mutate_manifest(temporary.path(), |manifest| {
        manifest["document_count"] = Value::from(1);
    });
    assert!(matches!(
        MethodologyCatalogV1::load(temporary.path(), &policy()),
        Err(MethodologyCatalogError::Contract(
            MethodologyContractError::InvalidField(_)
        ))
    ));
}

#[test]
fn methodology_index_matches_product_cwe_prerequisite_and_chain_tags_deterministically() {
    let catalog = load_fixture();
    let query = MethodologyQueryV1::new(
        [
            "prerequisite".into(),
            "authentication".into(),
            "wstg-athn".into(),
        ],
        8,
    )
    .unwrap();
    let first = catalog.query(&query, &policy()).unwrap();
    let second = catalog.query(&query, &policy()).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.hits.len(), 2);
    assert_eq!(first.hits[0].matched_tags, ["authentication", "wstg-athn"]);
    assert_eq!(first.hits[0].score_micros, 2_000_000);
    assert_eq!(first.omitted_hit_count, 0);
}

#[test]
fn methodology_skill_body_is_data_only_and_cannot_inject_tools_or_scope() {
    let catalog = load_fixture();
    let query = MethodologyQueryV1::new(["auth".into()], 1).unwrap();
    let result = catalog.query(&query, &policy()).unwrap();
    let rendered = serde_json::to_string(&result).unwrap();
    assert!(!rendered.contains("pentest_run"));
    assert!(!rendered.contains("browser_open"));
    assert!(!rendered.contains("ignore the host scope"));
    assert!(rendered.contains("methodology://sha256:"));
    assert!(!catalog.documents()[0].instruction_authority());
}

#[test]
fn methodology_hit_can_read_a_hash_bound_bounded_untrusted_excerpt() {
    let catalog = load_fixture();
    let query = MethodologyQueryV1::new(["auth".into()], 1).unwrap();
    let result = catalog.query(&query, &policy()).unwrap();

    let excerpt = catalog
        .read_untrusted_excerpt(&result.hits[0].document_id, 180)
        .unwrap();

    assert_eq!(excerpt.document_id, result.hits[0].document_id);
    assert_eq!(excerpt.content_sha256, result.hits[0].content_sha256);
    assert!(excerpt.untrusted_text.len() <= 180);
    assert!(excerpt
        .untrusted_text
        .contains("Authentication boundary review"));
    assert!(excerpt.truncated);
    assert!(!excerpt.instruction_authority());
    assert!(!excerpt.tool_authority());
    assert!(!excerpt.scope_authority());
    assert!(!excerpt.proof_authority());
}

#[test]
fn methodology_same_content_produces_same_document_and_corpus_hashes() {
    let catalog_a = load_fixture();
    let catalog_b = load_fixture();
    assert_eq!(
        catalog_a.manifest().corpus_id,
        catalog_b.manifest().corpus_id
    );
    assert_eq!(
        catalog_a.manifest().content_root_sha256,
        catalog_b.manifest().content_root_sha256
    );
    assert_eq!(
        catalog_a
            .documents()
            .iter()
            .map(|document| document.document_id.clone())
            .collect::<Vec<_>>(),
        catalog_b
            .documents()
            .iter()
            .map(|document| document.document_id.clone())
            .collect::<Vec<_>>()
    );

    let hash = sha256_bytes(b"same");
    let first = DeterministicDocumentId::derive("skills/same/SKILL.md", &hash);
    let second = DeterministicDocumentId::derive("skills/same/SKILL.md", &hash);
    assert_eq!(first, second);
    let descriptor = MethodologyDocumentDescriptorV1::validate(
        first.clone(),
        "skills/same/SKILL.md".into(),
        hash,
        ["auth".into()],
    )
    .unwrap();
    assert_eq!(descriptor.document_id, first);
}

#[test]
fn methodology_supersession_signature_revocation_and_license_policy_fail_closed() {
    for (field, value) in [
        ("signature_state", Value::String("unknown".into())),
        ("signature_state", Value::String("revoked".into())),
        (
            "superseded_at",
            Value::String("2026-08-03T00:00:00Z".into()),
        ),
    ] {
        let temporary = copy_fixture();
        mutate_manifest(temporary.path(), |manifest| {
            manifest[field] = value;
        });
        assert!(MethodologyCatalogV1::load(temporary.path(), &policy()).is_err());
    }

    let rejected_license = MethodologyTrustPolicyV1::new(1, ["MIT".to_string()]).unwrap();
    assert!(matches!(
        MethodologyCatalogV1::load(&fixture_root(), &rejected_license),
        Err(MethodologyCatalogError::Contract(
            MethodologyContractError::LicenseRejected(_)
        ))
    ));
    let stale_epoch = MethodologyTrustPolicyV1::new(2, ["Apache-2.0".to_string()]).unwrap();
    assert!(matches!(
        MethodologyCatalogV1::load(&fixture_root(), &stale_epoch),
        Err(MethodologyCatalogError::Contract(
            MethodologyContractError::StaleTrustEpoch { .. }
        ))
    ));
}

#[cfg(unix)]
#[test]
fn methodology_resolver_rejects_root_and_parent_symlink_escape() {
    use std::os::unix::fs::symlink;

    let real = copy_fixture();
    let parent = tempfile::tempdir().unwrap();
    let linked_root = parent.path().join("linked-corpus");
    symlink(real.path(), &linked_root).unwrap();
    assert!(matches!(
        MethodologyCatalogV1::load(&linked_root, &policy()),
        Err(MethodologyCatalogError::Security(_))
    ));

    let intermediate = copy_fixture();
    let skills = intermediate.path().join("skills");
    let real_skills = intermediate.path().join("skills-real");
    std::fs::rename(&skills, &real_skills).unwrap();
    symlink(&real_skills, &skills).unwrap();
    assert!(matches!(
        MethodologyCatalogV1::load(intermediate.path(), &policy()),
        Err(MethodologyCatalogError::Security(_))
    ));

    let escaped = copy_fixture();
    let outside = tempfile::tempdir().unwrap();
    let source = escaped.path().join("skills/auth-testing/SKILL.md");
    std::fs::remove_file(&source).unwrap();
    std::fs::write(
        outside.path().join("SKILL.md"),
        "---\nname: escape\ndescription: escape\n---\nbody\n",
    )
    .unwrap();
    symlink(outside.path().join("SKILL.md"), &source).unwrap();
    assert!(matches!(
        MethodologyCatalogV1::load(escaped.path(), &policy()),
        Err(MethodologyCatalogError::Security(_))
    ));
}

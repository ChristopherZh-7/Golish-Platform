//! Safe, content-addressed methodology catalog ingestion.
//!
//! Third-party `SKILL.md` frontmatter is validated by a deliberately separate,
//! data-only parser. Bodies remain untrusted data on disk; query results expose
//! immutable refs and hashes, never executable instructions or caller-controlled
//! authority.

use crate::SkillsError;
use chrono::{DateTime, Utc};
use golish_core::methodology_context::{
    methodology_result_set_sha256, sha256_bytes, DeterministicCorpusId, DeterministicDocumentId,
    MethodologyContractError, MethodologyCorpusIdentityMaterial, MethodologyCorpusManifestV1,
    MethodologyDocumentDescriptorV1, MethodologyHitV1, MethodologyQueryResultV1,
    MethodologyQueryV1, MethodologySignatureStateV1, MethodologySourceKindV1,
    MethodologyTrustPolicyV1, NewMethodologyCorpusManifestV1,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs::Metadata;
use std::path::{Component, Path, PathBuf};

#[cfg(unix)]
use std::ffi::{CStr, CString, OsString};
#[cfg(unix)]
use std::fs::File;
#[cfg(unix)]
use std::io::Read;
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};
#[cfg(unix)]
use std::os::unix::ffi::{OsStrExt, OsStringExt};

pub const METHODOLOGY_MANIFEST_FILE: &str = "manifest.json";
pub const MAX_METHODOLOGY_EXCERPT_BYTES: usize = 16_384;

#[derive(Debug, thiserror::Error)]
pub enum MethodologyCatalogError {
    #[error("methodology catalog I/O error: {0}")]
    Io(String),
    #[error("methodology catalog parse error: {0}")]
    Parse(String),
    #[error("methodology catalog security error: {0}")]
    Security(String),
    #[error(transparent)]
    Contract(#[from] MethodologyContractError),
}

impl From<SkillsError> for MethodologyCatalogError {
    fn from(value: SkillsError) -> Self {
        Self::Parse(value.to_string())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MethodologyManifestFileV1 {
    schema_version: String,
    corpus_id: String,
    source_kind: MethodologySourceKindV1,
    upstream_url: Option<String>,
    upstream_revision: String,
    license_spdx: String,
    license_text_sha256: String,
    signature_state: MethodologySignatureStateV1,
    trust_store_epoch: u64,
    document_count: u32,
    content_root_sha256: String,
    parser_contract_version: String,
    index_contract_version: String,
    ingested_at: DateTime<Utc>,
    superseded_at: Option<DateTime<Utc>>,
    documents: Vec<MethodologyManifestDocumentV1>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MethodologyManifestDocumentV1 {
    document_id: String,
    relative_path: String,
    content_sha256: String,
    tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct MethodologyRootMember<'a> {
    document_id: &'a str,
    relative_path: &'a str,
    content_sha256: &'a str,
    normalized_tags: &'a [String],
}

#[derive(Debug, Clone)]
pub struct MethodologyCatalogV1 {
    declared_root: PathBuf,
    manifest: MethodologyCorpusManifestV1,
    documents: Vec<MethodologyDocumentDescriptorV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MethodologyUntrustedExcerptV1 {
    pub document_id: DeterministicDocumentId,
    pub content_sha256: String,
    pub safe_excerpt_ref: String,
    pub untrusted_text: String,
    pub truncated: bool,
    instruction_authority: bool,
    tool_authority: bool,
    scope_authority: bool,
    proof_authority: bool,
}

impl MethodologyUntrustedExcerptV1 {
    pub const fn instruction_authority(&self) -> bool {
        self.instruction_authority
    }

    pub const fn tool_authority(&self) -> bool {
        self.tool_authority
    }

    pub const fn scope_authority(&self) -> bool {
        self.scope_authority
    }

    pub const fn proof_authority(&self) -> bool {
        self.proof_authority
    }
}

impl MethodologyCatalogV1 {
    pub fn load(
        declared_root: &Path,
        trust_policy: &MethodologyTrustPolicyV1,
    ) -> Result<Self, MethodologyCatalogError> {
        let root_metadata = std::fs::symlink_metadata(declared_root).map_err(|error| {
            MethodologyCatalogError::Io(format!(
                "cannot inspect corpus root {}: {error}",
                declared_root.display()
            ))
        })?;
        if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
            return Err(MethodologyCatalogError::Security(
                "corpus root must be a real directory, not a symlink".into(),
            ));
        }
        #[cfg(unix)]
        let anchored_root = AnchoredCorpusRoot::open(declared_root, &root_metadata)?;
        #[cfg(not(unix))]
        let canonical_root = std::fs::canonicalize(declared_root).map_err(|error| {
            MethodologyCatalogError::Io(format!(
                "cannot canonicalize corpus root {}: {error}",
                declared_root.display()
            ))
        })?;

        #[cfg(unix)]
        let manifest_bytes = anchored_root.read_regular(Path::new(METHODOLOGY_MANIFEST_FILE))?;
        #[cfg(not(unix))]
        let manifest_bytes = {
            let manifest_path = resolve_regular_file(
                declared_root,
                &canonical_root,
                Path::new(METHODOLOGY_MANIFEST_FILE),
            )?;
            read_identity_stable(&manifest_path)?
        };
        let input: MethodologyManifestFileV1 = serde_json::from_slice(&manifest_bytes)
            .map_err(|error| MethodologyCatalogError::Parse(error.to_string()))?;
        if input.schema_version != "methodology_corpus_manifest.v1" {
            return Err(MethodologyCatalogError::Parse(format!(
                "unsupported manifest schema {}",
                input.schema_version
            )));
        }
        if input.document_count as usize != input.documents.len() {
            return Err(MethodologyCatalogError::Contract(
                MethodologyContractError::InvalidField(
                    "manifest document_count does not match members".into(),
                ),
            ));
        }

        let mut documents = Vec::with_capacity(input.documents.len());
        let mut paths = BTreeSet::new();
        let mut ids = BTreeSet::new();
        for document in &input.documents {
            let relative_path = Path::new(&document.relative_path);
            validate_relative_path(relative_path)?;
            if !paths.insert(document.relative_path.clone()) {
                return Err(MethodologyCatalogError::Parse(format!(
                    "duplicate methodology path {}",
                    document.relative_path
                )));
            }
            #[cfg(unix)]
            let bytes = anchored_root.read_regular(relative_path)?;
            #[cfg(not(unix))]
            let bytes = {
                let resolved = resolve_regular_file(declared_root, &canonical_root, relative_path)?;
                read_identity_stable(&resolved)?
            };
            let actual_content_sha256 = sha256_bytes(&bytes);
            if actual_content_sha256 != document.content_sha256 {
                return Err(MethodologyCatalogError::Contract(
                    MethodologyContractError::IdentityMismatch {
                        kind: "content_sha256",
                        expected: actual_content_sha256,
                        actual: document.content_sha256.clone(),
                    },
                ));
            }
            let content = std::str::from_utf8(&bytes).map_err(|error| {
                MethodologyCatalogError::Parse(format!(
                    "{} is not UTF-8: {error}",
                    document.relative_path
                ))
            })?;
            if !parse_methodology_skill_md_data_only(content) {
                return Err(MethodologyCatalogError::Parse(format!(
                    "{} is not a valid SKILL.md document",
                    document.relative_path
                )));
            }
            let descriptor = MethodologyDocumentDescriptorV1::validate(
                DeterministicDocumentId::parse(document.document_id.clone())?,
                document.relative_path.clone(),
                document.content_sha256.clone(),
                document.tags.clone(),
            )?;
            if !ids.insert(descriptor.document_id.as_str().to_string()) {
                return Err(MethodologyCatalogError::Parse(format!(
                    "duplicate methodology document id {}",
                    descriptor.document_id.as_str()
                )));
            }
            documents.push(descriptor);
        }
        documents.sort_by(|left, right| {
            left.relative_path
                .cmp(&right.relative_path)
                .then_with(|| left.document_id.cmp(&right.document_id))
        });

        #[cfg(unix)]
        let discovered = discover_skill_documents(&anchored_root)?;
        #[cfg(not(unix))]
        let discovered = discover_skill_documents(declared_root, &canonical_root)?;
        if discovered != paths {
            return Err(MethodologyCatalogError::Security(format!(
                "manifest document set differs from on-disk SKILL.md set: declared={paths:?}, discovered={discovered:?}"
            )));
        }

        let actual_root = methodology_content_root_sha256(&documents);
        if actual_root != input.content_root_sha256 {
            return Err(MethodologyCatalogError::Contract(
                MethodologyContractError::IdentityMismatch {
                    kind: "content_root_sha256",
                    expected: actual_root,
                    actual: input.content_root_sha256.clone(),
                },
            ));
        }
        let identity = MethodologyCorpusIdentityMaterial {
            source_kind: input.source_kind,
            upstream_url: input.upstream_url.as_deref(),
            upstream_revision: &input.upstream_revision,
            license_spdx: &input.license_spdx,
            license_text_sha256: &input.license_text_sha256,
            document_count: input.document_count,
            content_root_sha256: &input.content_root_sha256,
            parser_contract_version: &input.parser_contract_version,
            index_contract_version: &input.index_contract_version,
        };
        let claimed_corpus_id = DeterministicCorpusId::parse(input.corpus_id)?;
        let expected_corpus_id = DeterministicCorpusId::derive(&identity);
        if expected_corpus_id != claimed_corpus_id {
            return Err(MethodologyCatalogError::Contract(
                MethodologyContractError::IdentityMismatch {
                    kind: "corpus_id",
                    expected: expected_corpus_id.as_str().to_string(),
                    actual: claimed_corpus_id.as_str().to_string(),
                },
            ));
        }
        let manifest = MethodologyCorpusManifestV1::validate(NewMethodologyCorpusManifestV1 {
            claimed_corpus_id,
            source_kind: input.source_kind,
            upstream_url: input.upstream_url,
            upstream_revision: input.upstream_revision,
            license_spdx: input.license_spdx,
            license_text_sha256: input.license_text_sha256,
            signature_state: input.signature_state,
            trust_store_epoch: input.trust_store_epoch,
            document_count: input.document_count,
            content_root_sha256: input.content_root_sha256,
            parser_contract_version: input.parser_contract_version,
            index_contract_version: input.index_contract_version,
            ingested_at: input.ingested_at,
            superseded_at: input.superseded_at,
        })?;
        manifest.authorize_for_query(trust_policy)?;
        Ok(Self {
            declared_root: declared_root.to_path_buf(),
            manifest,
            documents,
        })
    }

    pub fn manifest(&self) -> &MethodologyCorpusManifestV1 {
        &self.manifest
    }

    pub fn documents(&self) -> &[MethodologyDocumentDescriptorV1] {
        &self.documents
    }

    pub fn read_untrusted_excerpt(
        &self,
        document_id: &DeterministicDocumentId,
        max_bytes: usize,
    ) -> Result<MethodologyUntrustedExcerptV1, MethodologyCatalogError> {
        if max_bytes == 0 || max_bytes > MAX_METHODOLOGY_EXCERPT_BYTES {
            return Err(MethodologyCatalogError::Contract(
                MethodologyContractError::InvalidField(format!(
                    "methodology excerpt max_bytes must be between 1 and {MAX_METHODOLOGY_EXCERPT_BYTES}"
                )),
            ));
        }
        let descriptor = self
            .documents
            .iter()
            .find(|document| &document.document_id == document_id)
            .ok_or_else(|| {
                MethodologyCatalogError::Contract(MethodologyContractError::InvalidField(
                    "methodology excerpt document is not in the loaded corpus".into(),
                ))
            })?;
        let bytes =
            read_corpus_regular_file(&self.declared_root, Path::new(&descriptor.relative_path))?;
        let actual_content_sha256 = sha256_bytes(&bytes);
        if actual_content_sha256 != descriptor.content_sha256 {
            return Err(MethodologyCatalogError::Contract(
                MethodologyContractError::IdentityMismatch {
                    kind: "content_sha256",
                    expected: descriptor.content_sha256.clone(),
                    actual: actual_content_sha256,
                },
            ));
        }
        let content = std::str::from_utf8(&bytes).map_err(|error| {
            MethodologyCatalogError::Parse(format!(
                "{} is not UTF-8: {error}",
                descriptor.relative_path
            ))
        })?;
        let body = methodology_body_data(content).ok_or_else(|| {
            MethodologyCatalogError::Parse(format!(
                "{} no longer satisfies the data-only methodology parser",
                descriptor.relative_path
            ))
        })?;
        let mut excerpt_end = body.len().min(max_bytes);
        while excerpt_end > 0 && !body.is_char_boundary(excerpt_end) {
            excerpt_end -= 1;
        }
        let untrusted_text = body[..excerpt_end].trim_end().to_owned();
        Ok(MethodologyUntrustedExcerptV1 {
            document_id: descriptor.document_id.clone(),
            content_sha256: descriptor.content_sha256.clone(),
            safe_excerpt_ref: descriptor.safe_excerpt_ref.clone(),
            untrusted_text,
            truncated: excerpt_end < body.len(),
            instruction_authority: false,
            tool_authority: false,
            scope_authority: false,
            proof_authority: false,
        })
    }

    pub fn query(
        &self,
        query: &MethodologyQueryV1,
        trust_policy: &MethodologyTrustPolicyV1,
    ) -> Result<MethodologyQueryResultV1, MethodologyCatalogError> {
        self.manifest.authorize_for_query(trust_policy)?;
        let query_tags = query
            .normalized_tags
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let mut ranked = self
            .documents
            .iter()
            .filter_map(|document| {
                let matched_tags = document
                    .normalized_tags
                    .iter()
                    .filter(|tag| query_tags.contains(tag.as_str()))
                    .cloned()
                    .collect::<Vec<_>>();
                if matched_tags.is_empty() {
                    return None;
                }
                Some(MethodologyHitV1 {
                    corpus_id: self.manifest.corpus_id.clone(),
                    document_id: document.document_id.clone(),
                    relative_path: document.relative_path.clone(),
                    content_sha256: document.content_sha256.clone(),
                    safe_excerpt_ref: document.safe_excerpt_ref.clone(),
                    score_micros: i64::try_from(matched_tags.len())
                        .unwrap_or(i64::MAX)
                        .saturating_mul(1_000_000),
                    matched_tags,
                })
            })
            .collect::<Vec<_>>();
        ranked.sort_by(|left, right| {
            right
                .score_micros
                .cmp(&left.score_micros)
                .then_with(|| left.corpus_id.cmp(&right.corpus_id))
                .then_with(|| left.document_id.cmp(&right.document_id))
        });
        let total = ranked.len();
        ranked.truncate(query.top_k as usize);
        let omitted_hit_count =
            u32::try_from(total.saturating_sub(ranked.len())).unwrap_or(u32::MAX);
        let result_set_sha256 = methodology_result_set_sha256(&ranked);
        Ok(MethodologyQueryResultV1 {
            hits: ranked,
            omitted_hit_count,
            result_set_sha256,
        })
    }
}

fn parse_methodology_skill_md_data_only(content: &str) -> bool {
    parse_methodology_frontmatter_data_only(content)
}

fn methodology_frontmatter_data(content: &str) -> Option<&str> {
    if !content.starts_with("---") {
        return None;
    }
    let end_pos = content[3..].find("\n---")?;
    Some(content[3..3 + end_pos].trim())
}

fn methodology_body_data(content: &str) -> Option<&str> {
    let end_pos = content[3..].find("\n---")?;
    let body_start = 3 + end_pos + "\n---".len();
    Some(content[body_start..].trim_start_matches(['\r', '\n']))
}

fn parse_methodology_frontmatter_data_only(content: &str) -> bool {
    let Some(frontmatter_text) = methodology_frontmatter_data(content) else {
        return false;
    };
    let Ok(frontmatter) = serde_yaml::from_str::<serde_yaml::Value>(frontmatter_text) else {
        return false;
    };
    let Some(mapping) = frontmatter.as_mapping() else {
        return false;
    };
    let name_key = serde_yaml::Value::String("name".to_owned());
    let valid_name = mapping
        .get(&name_key)
        .and_then(serde_yaml::Value::as_str)
        .is_some_and(|name| !name.is_empty() && name.len() <= 256);
    let description_key = serde_yaml::Value::String("description".to_owned());
    let valid_description = match mapping.get(&description_key) {
        None | Some(serde_yaml::Value::Null) => true,
        Some(serde_yaml::Value::String(description)) => {
            !description.is_empty() && description.len() <= 4_096
        }
        _ => false,
    };
    valid_name && valid_description
}

pub fn methodology_content_root_sha256(documents: &[MethodologyDocumentDescriptorV1]) -> String {
    let mut members = documents
        .iter()
        .map(|document| MethodologyRootMember {
            document_id: document.document_id.as_str(),
            relative_path: &document.relative_path,
            content_sha256: &document.content_sha256,
            normalized_tags: &document.normalized_tags,
        })
        .collect::<Vec<_>>();
    members.sort_by(|left, right| {
        left.relative_path
            .cmp(right.relative_path)
            .then_with(|| left.document_id.cmp(right.document_id))
    });
    sha256_bytes(&serde_json::to_vec(&members).expect("root members are serializable"))
}

fn validate_relative_path(relative: &Path) -> Result<(), MethodologyCatalogError> {
    if relative.as_os_str().is_empty() || relative.is_absolute() {
        return Err(MethodologyCatalogError::Security(
            "methodology path must be non-empty and relative".into(),
        ));
    }
    for component in relative.components() {
        match component {
            Component::Normal(_) => {}
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(MethodologyCatalogError::Security(format!(
                    "methodology path contains forbidden component: {}",
                    relative.display()
                )))
            }
        }
    }
    Ok(())
}

fn read_corpus_regular_file(
    declared_root: &Path,
    relative: &Path,
) -> Result<Vec<u8>, MethodologyCatalogError> {
    let root_metadata = std::fs::symlink_metadata(declared_root).map_err(|error| {
        MethodologyCatalogError::Io(format!(
            "cannot inspect corpus root {}: {error}",
            declared_root.display()
        ))
    })?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(MethodologyCatalogError::Security(
            "corpus root must be a real directory, not a symlink".into(),
        ));
    }
    #[cfg(unix)]
    {
        AnchoredCorpusRoot::open(declared_root, &root_metadata)?.read_regular(relative)
    }
    #[cfg(not(unix))]
    {
        let canonical_root = std::fs::canonicalize(declared_root).map_err(|error| {
            MethodologyCatalogError::Io(format!(
                "cannot canonicalize corpus root {}: {error}",
                declared_root.display()
            ))
        })?;
        let resolved = resolve_regular_file(declared_root, &canonical_root, relative)?;
        read_identity_stable(&resolved)
    }
}

#[cfg(unix)]
const METHODOLOGY_DIRECTORY_FLAGS: libc::c_int =
    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW;

#[cfg(unix)]
struct AnchoredCorpusRoot {
    directory: File,
}

#[cfg(unix)]
impl AnchoredCorpusRoot {
    fn open(path: &Path, expected: &Metadata) -> Result<Self, MethodologyCatalogError> {
        use std::os::unix::fs::MetadataExt;

        let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
            MethodologyCatalogError::Security(
                "methodology corpus root contains an embedded NUL".into(),
            )
        })?;
        let raw = unsafe { libc::open(path.as_ptr(), METHODOLOGY_DIRECTORY_FLAGS) };
        if raw < 0 {
            return Err(open_error(
                Path::new("."),
                std::io::Error::last_os_error(),
                "corpus root",
            ));
        }
        let directory = unsafe { File::from_raw_fd(raw) };
        let actual = directory.metadata().map_err(|error| {
            MethodologyCatalogError::Io(format!("cannot inspect opened corpus root: {error}"))
        })?;
        if !actual.is_dir() || actual.dev() != expected.dev() || actual.ino() != expected.ino() {
            return Err(MethodologyCatalogError::Security(
                "methodology corpus root binding changed while opening".into(),
            ));
        }
        Ok(Self { directory })
    }

    fn open_directory(&self, relative: &Path) -> Result<File, MethodologyCatalogError> {
        validate_relative_path(relative)?;
        let mut current = self.directory.try_clone().map_err(|error| {
            MethodologyCatalogError::Io(format!("cannot clone corpus root handle: {error}"))
        })?;
        for component in relative.components() {
            let Component::Normal(name) = component else {
                unreachable!("validated above")
            };
            current = open_directory_component(&current, name.as_bytes(), relative)?;
        }
        Ok(current)
    }

    fn read_regular(&self, relative: &Path) -> Result<Vec<u8>, MethodologyCatalogError> {
        use std::os::unix::fs::MetadataExt;

        validate_relative_path(relative)?;
        let components = relative
            .components()
            .map(|component| {
                let Component::Normal(name) = component else {
                    unreachable!("validated above")
                };
                name
            })
            .collect::<Vec<_>>();
        let (file_name, parent_components) = components
            .split_last()
            .ok_or_else(|| MethodologyCatalogError::Security("methodology path is empty".into()))?;
        let mut parent = self.directory.try_clone().map_err(|error| {
            MethodologyCatalogError::Io(format!("cannot clone corpus root handle: {error}"))
        })?;
        for component in parent_components {
            parent = open_directory_component(&parent, component.as_bytes(), relative)?;
        }

        let name = c_path_component(file_name.as_bytes(), relative)?;
        let named_before = stat_at(&parent, &name, relative)?;
        if entry_kind(&named_before) == AnchoredEntryKind::Symlink {
            return Err(MethodologyCatalogError::Security(format!(
                "methodology path contains symlink: {}",
                relative.display()
            )));
        }
        if entry_kind(&named_before) != AnchoredEntryKind::Regular {
            return Err(MethodologyCatalogError::Security(format!(
                "methodology document is not a regular file: {}",
                relative.display()
            )));
        }

        let raw = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if raw < 0 {
            return Err(open_error(
                relative,
                std::io::Error::last_os_error(),
                "document",
            ));
        }
        let file = unsafe { File::from_raw_fd(raw) };
        let before = file.metadata().map_err(|error| {
            MethodologyCatalogError::Io(format!(
                "cannot inspect opened methodology document {}: {error}",
                relative.display()
            ))
        })?;
        if !before.is_file()
            || before.dev() != named_before.st_dev as u64
            || before.ino() != named_before.st_ino
        {
            return Err(MethodologyCatalogError::Security(format!(
                "methodology document binding changed while opening: {}",
                relative.display()
            )));
        }
        let mut bytes = Vec::new();
        let mut reader = &file;
        reader.read_to_end(&mut bytes).map_err(|error| {
            MethodologyCatalogError::Io(format!(
                "cannot read methodology document {}: {error}",
                relative.display()
            ))
        })?;
        let after = file.metadata().map_err(|error| {
            MethodologyCatalogError::Io(format!(
                "cannot re-inspect methodology document {}: {error}",
                relative.display()
            ))
        })?;
        let named_after = stat_at(&parent, &name, relative)?;
        if entry_kind(&named_after) != AnchoredEntryKind::Regular
            || named_after.st_dev != named_before.st_dev
            || named_after.st_ino != named_before.st_ino
            || !same_file_identity(&before, &after)
            || after.len() != bytes.len() as u64
        {
            return Err(MethodologyCatalogError::Security(format!(
                "methodology document changed while being read: {}",
                relative.display()
            )));
        }
        Ok(bytes)
    }
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AnchoredEntryKind {
    Directory,
    Regular,
    Symlink,
    Other,
}

#[cfg(unix)]
fn entry_kind(stat: &libc::stat) -> AnchoredEntryKind {
    match stat.st_mode & libc::S_IFMT {
        libc::S_IFDIR => AnchoredEntryKind::Directory,
        libc::S_IFREG => AnchoredEntryKind::Regular,
        libc::S_IFLNK => AnchoredEntryKind::Symlink,
        _ => AnchoredEntryKind::Other,
    }
}

#[cfg(unix)]
fn c_path_component(name: &[u8], relative: &Path) -> Result<CString, MethodologyCatalogError> {
    if name.is_empty() || name == b"." || name == b".." || name.contains(&b'/') {
        return Err(MethodologyCatalogError::Security(format!(
            "invalid methodology path component: {}",
            relative.display()
        )));
    }
    CString::new(name).map_err(|_| {
        MethodologyCatalogError::Security(format!(
            "methodology path contains an embedded NUL: {}",
            relative.display()
        ))
    })
}

#[cfg(unix)]
fn stat_at(
    directory: &File,
    name: &CString,
    relative: &Path,
) -> Result<libc::stat, MethodologyCatalogError> {
    let mut stat = unsafe { std::mem::zeroed() };
    let result = unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            name.as_ptr(),
            &mut stat,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result == 0 {
        return Ok(stat);
    }
    Err(MethodologyCatalogError::Io(format!(
        "cannot inspect methodology path {}: {}",
        relative.display(),
        std::io::Error::last_os_error()
    )))
}

#[cfg(unix)]
fn open_directory_component(
    parent: &File,
    name_bytes: &[u8],
    relative: &Path,
) -> Result<File, MethodologyCatalogError> {
    use std::os::unix::fs::MetadataExt;

    let name = c_path_component(name_bytes, relative)?;
    let named = stat_at(parent, &name, relative)?;
    if entry_kind(&named) == AnchoredEntryKind::Symlink {
        return Err(MethodologyCatalogError::Security(format!(
            "methodology path contains symlink: {}",
            relative.display()
        )));
    }
    if entry_kind(&named) != AnchoredEntryKind::Directory {
        return Err(MethodologyCatalogError::Security(format!(
            "methodology parent is not a directory: {}",
            relative.display()
        )));
    }
    let raw = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            METHODOLOGY_DIRECTORY_FLAGS,
        )
    };
    if raw < 0 {
        return Err(open_error(
            relative,
            std::io::Error::last_os_error(),
            "directory",
        ));
    }
    let directory = unsafe { File::from_raw_fd(raw) };
    let metadata = directory.metadata().map_err(|error| {
        MethodologyCatalogError::Io(format!(
            "cannot inspect opened methodology directory {}: {error}",
            relative.display()
        ))
    })?;
    if !metadata.is_dir() || metadata.dev() != named.st_dev as u64 || metadata.ino() != named.st_ino
    {
        return Err(MethodologyCatalogError::Security(format!(
            "methodology directory binding changed while opening: {}",
            relative.display()
        )));
    }
    Ok(directory)
}

#[cfg(unix)]
fn open_error(relative: &Path, error: std::io::Error, kind: &str) -> MethodologyCatalogError {
    if matches!(error.raw_os_error(), Some(code) if code == libc::ELOOP || code == libc::ENOTDIR) {
        return MethodologyCatalogError::Security(format!(
            "methodology {kind} refused a symlink or non-directory component at {}: {error}",
            relative.display()
        ));
    }
    MethodologyCatalogError::Io(format!(
        "cannot open methodology {kind} {}: {error}",
        relative.display()
    ))
}

#[cfg(not(unix))]
fn resolve_regular_file(
    declared_root: &Path,
    canonical_root: &Path,
    relative: &Path,
) -> Result<PathBuf, MethodologyCatalogError> {
    validate_relative_path(relative)?;
    let mut current = declared_root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            unreachable!("validated above")
        };
        current.push(part);
        let metadata = std::fs::symlink_metadata(&current).map_err(|error| {
            MethodologyCatalogError::Io(format!("cannot inspect {}: {error}", current.display()))
        })?;
        if metadata.file_type().is_symlink() {
            return Err(MethodologyCatalogError::Security(format!(
                "methodology path contains symlink: {}",
                current.display()
            )));
        }
    }
    let canonical = std::fs::canonicalize(&current).map_err(|error| {
        MethodologyCatalogError::Io(format!(
            "cannot canonicalize {}: {error}",
            current.display()
        ))
    })?;
    if !canonical.starts_with(canonical_root) {
        return Err(MethodologyCatalogError::Security(format!(
            "methodology path escapes corpus root: {}",
            relative.display()
        )));
    }
    let metadata = std::fs::metadata(&canonical).map_err(|error| {
        MethodologyCatalogError::Io(format!("cannot inspect {}: {error}", canonical.display()))
    })?;
    if !metadata.is_file() {
        return Err(MethodologyCatalogError::Security(format!(
            "methodology document is not a regular file: {}",
            relative.display()
        )));
    }
    Ok(canonical)
}

#[cfg(not(unix))]
fn read_identity_stable(path: &Path) -> Result<Vec<u8>, MethodologyCatalogError> {
    let before = std::fs::metadata(path).map_err(|error| {
        MethodologyCatalogError::Io(format!("cannot inspect {}: {error}", path.display()))
    })?;
    let bytes = std::fs::read(path).map_err(|error| {
        MethodologyCatalogError::Io(format!("cannot read {}: {error}", path.display()))
    })?;
    let after = std::fs::metadata(path).map_err(|error| {
        MethodologyCatalogError::Io(format!("cannot re-inspect {}: {error}", path.display()))
    })?;
    if !same_file_identity(&before, &after) || after.len() != bytes.len() as u64 {
        return Err(MethodologyCatalogError::Security(format!(
            "methodology document changed while being read: {}",
            path.display()
        )));
    }
    Ok(bytes)
}

#[cfg(unix)]
fn same_file_identity(left: &Metadata, right: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.dev() == right.dev()
        && left.ino() == right.ino()
        && left.len() == right.len()
        && left.mtime() == right.mtime()
        && left.mtime_nsec() == right.mtime_nsec()
}

#[cfg(not(unix))]
fn same_file_identity(left: &Metadata, right: &Metadata) -> bool {
    left.len() == right.len()
        && left.modified().ok() == right.modified().ok()
        && left.created().ok() == right.created().ok()
}

#[cfg(unix)]
struct DirectoryStream(*mut libc::DIR);

#[cfg(unix)]
impl Drop for DirectoryStream {
    fn drop(&mut self) {
        unsafe {
            libc::closedir(self.0);
        }
    }
}

#[cfg(unix)]
fn directory_entries(directory: &File) -> Result<Vec<OsString>, MethodologyCatalogError> {
    let raw = directory
        .try_clone()
        .map_err(|error| {
            MethodologyCatalogError::Io(format!("cannot clone methodology directory: {error}"))
        })?
        .into_raw_fd();
    let stream = unsafe { libc::fdopendir(raw) };
    if stream.is_null() {
        unsafe {
            libc::close(raw);
        }
        return Err(MethodologyCatalogError::Io(format!(
            "cannot enumerate methodology directory: {}",
            std::io::Error::last_os_error()
        )));
    }
    let stream = DirectoryStream(stream);
    let mut entries = Vec::new();
    loop {
        let entry = unsafe { libc::readdir(stream.0) };
        if entry.is_null() {
            break;
        }
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if name == b"." || name == b".." {
            continue;
        }
        entries.push(OsString::from_vec(name.to_vec()));
    }
    entries.sort();
    Ok(entries)
}

#[cfg(unix)]
fn discover_skill_documents(
    root: &AnchoredCorpusRoot,
) -> Result<BTreeSet<String>, MethodologyCatalogError> {
    let skills = root.open_directory(Path::new("skills"))?;
    let mut pending = vec![(skills, "skills".to_string())];
    let mut discovered = BTreeSet::new();
    while let Some((directory, prefix)) = pending.pop() {
        for entry_name in directory_entries(&directory)? {
            let Some(name_text) = entry_name.to_str() else {
                return Err(MethodologyCatalogError::Security(format!(
                    "methodology tree contains a non-UTF-8 entry under {prefix}"
                )));
            };
            let relative = format!("{prefix}/{name_text}");
            let relative_path = Path::new(&relative);
            let name = c_path_component(entry_name.as_bytes(), relative_path)?;
            let stat = stat_at(&directory, &name, relative_path)?;
            match entry_kind(&stat) {
                AnchoredEntryKind::Symlink => {
                    return Err(MethodologyCatalogError::Security(format!(
                        "methodology tree contains symlink: {relative}"
                    )))
                }
                AnchoredEntryKind::Directory => {
                    let child =
                        open_directory_component(&directory, name.to_bytes(), relative_path)?;
                    pending.push((child, relative));
                }
                AnchoredEntryKind::Regular => {
                    if name_text == "SKILL.md" {
                        discovered.insert(relative);
                    }
                }
                AnchoredEntryKind::Other => {
                    return Err(MethodologyCatalogError::Security(format!(
                        "methodology tree contains non-regular entry: {relative}"
                    )))
                }
            }
        }
    }
    Ok(discovered)
}

#[cfg(not(unix))]
fn discover_skill_documents(
    declared_root: &Path,
    canonical_root: &Path,
) -> Result<BTreeSet<String>, MethodologyCatalogError> {
    let skills_root = declared_root.join("skills");
    let metadata = std::fs::symlink_metadata(&skills_root).map_err(|error| {
        MethodologyCatalogError::Io(format!(
            "cannot inspect methodology skills root {}: {error}",
            skills_root.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(MethodologyCatalogError::Security(
            "methodology skills root must be a real directory".into(),
        ));
    }
    let mut pending = vec![skills_root];
    let mut discovered = BTreeSet::new();
    while let Some(directory) = pending.pop() {
        let mut entries = std::fs::read_dir(&directory)
            .map_err(|error| {
                MethodologyCatalogError::Io(format!("cannot list {}: {error}", directory.display()))
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| MethodologyCatalogError::Io(error.to_string()))?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let metadata = entry.metadata().map_err(|error| {
                MethodologyCatalogError::Io(format!("cannot inspect {}: {error}", path.display()))
            })?;
            let file_type = entry.file_type().map_err(|error| {
                MethodologyCatalogError::Io(format!(
                    "cannot inspect type of {}: {error}",
                    path.display()
                ))
            })?;
            if file_type.is_symlink() {
                return Err(MethodologyCatalogError::Security(format!(
                    "methodology tree contains symlink: {}",
                    path.display()
                )));
            }
            if metadata.is_dir() {
                pending.push(path);
                continue;
            }
            if !metadata.is_file() {
                return Err(MethodologyCatalogError::Security(format!(
                    "methodology tree contains non-regular entry: {}",
                    path.display()
                )));
            }
            if path.file_name().and_then(|value| value.to_str()) != Some("SKILL.md") {
                continue;
            }
            let canonical = std::fs::canonicalize(&path).map_err(|error| {
                MethodologyCatalogError::Io(format!(
                    "cannot canonicalize {}: {error}",
                    path.display()
                ))
            })?;
            if !canonical.starts_with(canonical_root) {
                return Err(MethodologyCatalogError::Security(format!(
                    "methodology document escapes root: {}",
                    path.display()
                )));
            }
            let relative = path
                .strip_prefix(declared_root)
                .map_err(|_| {
                    MethodologyCatalogError::Security(
                        "methodology document is outside declared root".into(),
                    )
                })?
                .to_string_lossy()
                .replace('\\', "/");
            discovered.insert(relative);
        }
    }
    Ok(discovered)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn methodology_frontmatter_accepts_bounded_third_party_metadata_without_skill_authority() {
        let long_name = "a".repeat(128);
        let content = format!(
            "---\nname: {long_name}\ndescription: third-party methodology\ntags: [auth]\ncustom_field: ignored-data\n---\nrun a tool\n"
        );
        assert!(parse_methodology_skill_md_data_only(&content));
        assert!(parse_methodology_skill_md_data_only(
            "---\nname: upstream-empty-description\ndescription:\ntags: [auth]\n---\nmethodology body"
        ));
        assert!(parse_methodology_skill_md_data_only(
            "---\nname: only-name\n---\nbody"
        ));
        assert!(!parse_methodology_skill_md_data_only(&format!(
            "---\nname: {}\ndescription: too long\n---\nbody",
            "x".repeat(257)
        )));
    }

    #[test]
    fn deterministic_root_is_order_independent() {
        let a_hash = sha256_bytes(b"a");
        let b_hash = sha256_bytes(b"b");
        let a = MethodologyDocumentDescriptorV1::validate(
            DeterministicDocumentId::derive("skills/a/SKILL.md", &a_hash),
            "skills/a/SKILL.md".into(),
            a_hash,
            ["auth".into()],
        )
        .unwrap();
        let b = MethodologyDocumentDescriptorV1::validate(
            DeterministicDocumentId::derive("skills/b/SKILL.md", &b_hash),
            "skills/b/SKILL.md".into(),
            b_hash,
            ["configuration".into()],
        )
        .unwrap();
        assert_eq!(
            methodology_content_root_sha256(&[a.clone(), b.clone()]),
            methodology_content_root_sha256(&[b, a])
        );
    }
}

//! Race-safe Unix report-artifact storage.
//!
//! Every project path component is opened from `/` with `openat(2)`,
//! `O_DIRECTORY`, and `O_NOFOLLOW`. Once opened, all file operations use those
//! directory descriptors; path bindings are revalidated with
//! `fstatat(AT_SYMLINK_NOFOLLOW)` before and after mutation.

use std::collections::{BTreeSet, HashMap};
use std::ffi::{CStr, CString};
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

use super::crud_ops::{
    sanitize_report_path_component, sha256_hex, ReportArtifactFormat, ReportArtifactGcOutcome,
    ReservedReportArtifact, StagedReportArtifact, StoredReportArtifact,
};

const DIRECTORY_FLAGS: libc::c_int =
    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

impl FileIdentity {
    fn for_file(file: &File) -> Result<Self> {
        let metadata = file.metadata()?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }

    fn for_stat(stat: &libc::stat) -> Self {
        Self {
            device: stat.st_dev as u64,
            inode: stat.st_ino,
        }
    }
}

struct DirectoryBinding {
    parent: File,
    name: CString,
    identity: FileIdentity,
}

impl DirectoryBinding {
    fn try_clone(&self) -> Result<Self> {
        Ok(Self {
            parent: self.parent.try_clone()?,
            name: self.name.clone(),
            identity: self.identity,
        })
    }
}

struct AnchoredDirectory {
    file: File,
    bindings: Vec<DirectoryBinding>,
}

impl AnchoredDirectory {
    fn open_project_root(path: &Path) -> Result<Self> {
        if !path.is_absolute() {
            anyhow::bail!("report_project_root_not_absolute");
        }
        let slash = CString::new("/").expect("slash has no NUL");
        let raw = unsafe { libc::open(slash.as_ptr(), DIRECTORY_FLAGS) };
        if raw < 0 {
            return Err(std::io::Error::last_os_error())
                .context("report_project_root_inaccessible");
        }
        let mut current = Self {
            file: unsafe { File::from_raw_fd(raw) },
            bindings: Vec::new(),
        };
        for component in path.components() {
            match component {
                Component::RootDir => {}
                Component::Normal(component) => {
                    let name = c_component_bytes(component.as_bytes())?;
                    current = current
                        .child_directory_c(&name, false)?
                        .ok_or_else(|| anyhow::anyhow!("report_project_root_inaccessible"))?;
                }
                _ => anyhow::bail!("report_project_root_invalid"),
            }
        }
        #[cfg(test)]
        super::crud_ops::run_report_fs_after_root_check_hook();
        current.verify_bindings()?;
        Ok(current)
    }

    fn try_clone(&self) -> Result<Self> {
        Ok(Self {
            file: self.file.try_clone()?,
            bindings: self
                .bindings
                .iter()
                .map(DirectoryBinding::try_clone)
                .collect::<Result<_>>()?,
        })
    }

    fn identity(&self) -> Result<FileIdentity> {
        FileIdentity::for_file(&self.file)
    }

    fn child_directory(&self, name: &str, create: bool) -> Result<Option<Self>> {
        self.child_directory_c(&c_component(name)?, create)
    }

    fn child_directory_c(&self, name: &CString, create: bool) -> Result<Option<Self>> {
        let mut raw =
            unsafe { libc::openat(self.file.as_raw_fd(), name.as_ptr(), DIRECTORY_FLAGS) };
        if raw < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::NotFound && !create {
                return Ok(None);
            }
            if error.kind() == std::io::ErrorKind::NotFound && create {
                let created =
                    unsafe { libc::mkdirat(self.file.as_raw_fd(), name.as_ptr(), libc::S_IRWXU) };
                if created < 0 {
                    let create_error = std::io::Error::last_os_error();
                    if create_error.kind() != std::io::ErrorKind::AlreadyExists {
                        return Err(create_error)
                            .context("report_artifact_directory_create_failed");
                    }
                }
                raw =
                    unsafe { libc::openat(self.file.as_raw_fd(), name.as_ptr(), DIRECTORY_FLAGS) };
            }
        }
        if raw < 0 {
            return Err(std::io::Error::last_os_error())
                .context("report_artifact_directory_open_failed");
        }
        let file = unsafe { File::from_raw_fd(raw) };
        let identity = FileIdentity::for_file(&file)?;
        let mut bindings = self
            .bindings
            .iter()
            .map(DirectoryBinding::try_clone)
            .collect::<Result<Vec<_>>>()?;
        bindings.push(DirectoryBinding {
            parent: self.file.try_clone()?,
            name: name.clone(),
            identity,
        });
        let child = Self { file, bindings };
        child.verify_bindings()?;
        Ok(Some(child))
    }

    fn descendant(&self, components: &[&str], create: bool) -> Result<Option<Self>> {
        let mut current = self.try_clone()?;
        for component in components {
            let Some(child) = current.child_directory(component, create)? else {
                return Ok(None);
            };
            current = child;
        }
        Ok(Some(current))
    }

    fn verify_bindings(&self) -> Result<()> {
        for binding in &self.bindings {
            let stat = stat_at(&binding.parent, &binding.name)?
                .ok_or_else(|| anyhow::anyhow!("report_artifact_path_binding_changed"))?;
            match kind_from_stat(&stat) {
                EntryKind::Symlink => anyhow::bail!("report_artifact_symlink_forbidden"),
                EntryKind::Directory => {}
                EntryKind::Regular | EntryKind::Other => {
                    anyhow::bail!("report_artifact_path_not_directory")
                }
            }
            if FileIdentity::for_stat(&stat) != binding.identity {
                anyhow::bail!("report_artifact_path_binding_changed");
            }
        }
        Ok(())
    }

    fn sync(&self) -> Result<()> {
        self.file.sync_all()?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EntryKind {
    Directory,
    Regular,
    Symlink,
    Other,
}

fn kind_from_stat(stat: &libc::stat) -> EntryKind {
    match stat.st_mode & libc::S_IFMT {
        libc::S_IFDIR => EntryKind::Directory,
        libc::S_IFREG => EntryKind::Regular,
        libc::S_IFLNK => EntryKind::Symlink,
        _ => EntryKind::Other,
    }
}

fn c_component(name: &str) -> Result<CString> {
    c_component_bytes(name.as_bytes())
}

fn c_component_bytes(name: &[u8]) -> Result<CString> {
    if name.is_empty() || name == b"." || name == b".." || name.contains(&b'/') {
        anyhow::bail!("report_artifact_path_component_invalid");
    }
    CString::new(name).context("report_artifact_path_component_invalid")
}

fn stat_at(directory: &File, name: &CString) -> Result<Option<libc::stat>> {
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
        return Ok(Some(stat));
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::NotFound {
        Ok(None)
    } else {
        Err(error).context("report_artifact_stat_failed")
    }
}

fn open_regular(
    directory: &AnchoredDirectory,
    name: &str,
    flags: libc::c_int,
    mode: libc::mode_t,
) -> Result<Option<File>> {
    directory.verify_bindings()?;
    let name = c_component(name)?;
    let raw = unsafe {
        libc::openat(
            directory.file.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            mode as libc::c_uint,
        )
    };
    if raw < 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::NotFound {
            return Ok(None);
        }
        return Err(error).context("report_artifact_file_open_failed");
    }
    let file = unsafe { File::from_raw_fd(raw) };
    if !file.metadata()?.is_file() {
        anyhow::bail!("report_artifact_path_not_file");
    }
    Ok(Some(file))
}

fn verify_named_file(directory: &AnchoredDirectory, name: &str, file: &File) -> Result<()> {
    directory.verify_bindings()?;
    let stat = stat_at(&directory.file, &c_component(name)?)?
        .ok_or_else(|| anyhow::anyhow!("report_artifact_file_missing"))?;
    if kind_from_stat(&stat) == EntryKind::Symlink {
        anyhow::bail!("report_artifact_symlink_forbidden");
    }
    if kind_from_stat(&stat) != EntryKind::Regular {
        anyhow::bail!("report_artifact_path_not_file");
    }
    if FileIdentity::for_stat(&stat) != FileIdentity::for_file(file)? {
        anyhow::bail!("report_artifact_path_binding_changed");
    }
    Ok(())
}

fn read_all(mut file: &File) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn unlink_regular(directory: &AnchoredDirectory, name: &str, file: &File) -> Result<bool> {
    verify_named_file(directory, name, file)?;
    let name = c_component(name)?;
    let result = unsafe { libc::unlinkat(directory.file.as_raw_fd(), name.as_ptr(), 0) };
    if result == 0 {
        directory.sync()?;
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::NotFound {
        Ok(false)
    } else {
        Err(error).context("report_artifact_unlink_failed")
    }
}

fn validate_sha256(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        anyhow::bail!("report_artifact_sha256_invalid");
    }
    Ok(())
}

fn artifact_filename(sha256: &str, format: ReportArtifactFormat) -> Result<String> {
    validate_sha256(sha256)?;
    Ok(format!("{sha256}.{}", format.extension()))
}

fn parse_artifact_filename(name: &str) -> Result<(String, ReportArtifactFormat)> {
    let (sha256, format) = if let Some(sha256) = name.strip_suffix(".md") {
        (sha256, ReportArtifactFormat::Markdown)
    } else if let Some(sha256) = name.strip_suffix(".json") {
        (sha256, ReportArtifactFormat::Json)
    } else {
        anyhow::bail!("report_artifact_filename_invalid");
    };
    validate_sha256(sha256)?;
    Ok((sha256.to_string(), format))
}

fn expected_staging_key(revision_id: &str, filename: &str) -> String {
    format!(".golish/reports/.staging/{revision_id}/{filename}")
}

fn expected_content_key(filename: &str) -> String {
    format!("sha256/{filename}")
}

fn expected_storage_path(content_key: &str) -> String {
    format!(".golish/reports/blobs/{content_key}")
}

fn validate_staged(staged: &StagedReportArtifact) -> Result<String> {
    let revision_id = sanitize_report_path_component(&staged.revision_id)?;
    let filename = artifact_filename(&staged.sha256, staged.format)?;
    if staged.staging_key != expected_staging_key(&revision_id, &filename) {
        anyhow::bail!("report_staging_key_invalid");
    }
    Ok(filename)
}

fn validate_stored(artifact: &StoredReportArtifact) -> Result<String> {
    let filename = artifact_filename(&artifact.sha256, artifact.format)?;
    let content_key = expected_content_key(&filename);
    if artifact.content_key != content_key
        || artifact.storage_path != expected_storage_path(&content_key)
    {
        anyhow::bail!("report_blob_key_invalid");
    }
    Ok(filename)
}

static IN_PROCESS_LOCKS: OnceLock<Mutex<HashMap<String, Weak<AsyncMutex<()>>>>> = OnceLock::new();

fn in_process_lock(key: &str) -> Arc<AsyncMutex<()>> {
    let mut locks = IN_PROCESS_LOCKS
        .get_or_init(Default::default)
        .lock()
        .expect("report content lock map poisoned");
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(key).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(AsyncMutex::new(()));
    locks.insert(key.to_string(), Arc::downgrade(&lock));
    lock
}

struct PreparedContentLock {
    directory: AnchoredDirectory,
    filename: String,
    content_key: String,
    root_identity: FileIdentity,
}

pub(super) struct ContentKeyReservation {
    directory: AnchoredDirectory,
    filename: String,
    file: File,
    content_key: String,
    root_identity: FileIdentity,
    _in_process: OwnedMutexGuard<()>,
}

impl ContentKeyReservation {
    fn verify(&self) -> Result<()> {
        self.directory.verify_bindings()?;
        verify_named_file(&self.directory, &self.filename, &self.file)
    }
}

impl Drop for ContentKeyReservation {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

fn prepare_content_lock(project_root: &Path, content_key: &str) -> Result<PreparedContentLock> {
    let root = AnchoredDirectory::open_project_root(project_root)?;
    let root_identity = root.identity()?;
    let directory = root
        .descendant(&[".golish", "reports", ".locks", "sha256"], true)?
        .ok_or_else(|| anyhow::anyhow!("report_artifact_lock_directory_missing"))?;
    let (_, filename) = content_key
        .split_once('/')
        .filter(|(algorithm, _)| *algorithm == "sha256")
        .ok_or_else(|| anyhow::anyhow!("report_artifact_content_key_invalid"))?;
    parse_artifact_filename(filename)?;
    Ok(PreparedContentLock {
        directory,
        filename: format!("{filename}.lock"),
        content_key: content_key.to_string(),
        root_identity,
    })
}

fn finish_content_lock(
    prepared: PreparedContentLock,
    in_process: OwnedMutexGuard<()>,
) -> Result<ContentKeyReservation> {
    prepared.directory.verify_bindings()?;
    let file = open_regular(
        &prepared.directory,
        &prepared.filename,
        libc::O_RDWR | libc::O_CREAT,
        libc::S_IRUSR | libc::S_IWUSR,
    )?
    .ok_or_else(|| anyhow::anyhow!("report_artifact_lock_open_failed"))?;
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(std::io::Error::last_os_error()).context("report_artifact_lock_failed");
    }
    let reservation = ContentKeyReservation {
        directory: prepared.directory,
        filename: prepared.filename,
        file,
        content_key: prepared.content_key,
        root_identity: prepared.root_identity,
        _in_process: in_process,
    };
    reservation.verify()?;
    Ok(reservation)
}

async fn acquire_content_lock(
    project_root: PathBuf,
    content_key: String,
) -> Result<ContentKeyReservation> {
    let prepared = tokio::task::spawn_blocking({
        let content_key = content_key.clone();
        move || prepare_content_lock(&project_root, &content_key)
    })
    .await
    .context("report_artifact_lock_prepare_join_failed")??;
    let process_key = format!(
        "{}:{}:{}",
        prepared.root_identity.device, prepared.root_identity.inode, content_key
    );
    let in_process = in_process_lock(&process_key).lock_owned().await;
    tokio::task::spawn_blocking(move || finish_content_lock(prepared, in_process))
        .await
        .context("report_artifact_lock_join_failed")?
}

fn ensure_reservation_root(
    root: &AnchoredDirectory,
    content_key: &str,
    reservation: &ContentKeyReservation,
) -> Result<()> {
    if root.identity()? != reservation.root_identity || content_key != reservation.content_key {
        anyhow::bail!("report_artifact_lock_scope_mismatch");
    }
    reservation.verify()
}

fn stage_blocking(
    project_root: &Path,
    revision_id: String,
    format: ReportArtifactFormat,
    bytes: Vec<u8>,
    reservation: &ContentKeyReservation,
) -> Result<StagedReportArtifact> {
    let root = AnchoredDirectory::open_project_root(project_root)?;
    let sha256 = sha256_hex(&bytes);
    let filename = artifact_filename(&sha256, format)?;
    let content_key = expected_content_key(&filename);
    ensure_reservation_root(&root, &content_key, reservation)?;
    let revision_directory = root
        .descendant(&[".golish", "reports", ".staging", &revision_id], true)?
        .ok_or_else(|| anyhow::anyhow!("report_staging_path_invalid"))?;

    #[cfg(test)]
    super::crud_ops::run_report_fs_after_parent_open_hook();

    revision_directory.verify_bindings()?;
    let mut created = false;
    let file = match open_regular(
        &revision_directory,
        &filename,
        libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
        libc::S_IRUSR | libc::S_IWUSR,
    ) {
        Ok(Some(mut file)) => {
            created = true;
            file.write_all(&bytes)?;
            file.sync_all()?;
            file
        }
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::AlreadyExists) =>
        {
            let existing = open_regular(&revision_directory, &filename, libc::O_RDWR, 0)?
                .ok_or_else(|| anyhow::anyhow!("report_staging_identity_conflict"))?;
            let existing_bytes = read_all(&existing)?;
            if sha256_hex(&existing_bytes) != sha256 || existing_bytes.len() != bytes.len() {
                anyhow::bail!("report_staging_identity_conflict");
            }
            existing
        }
        Ok(None) => anyhow::bail!("report_staging_write_failed"),
        Err(error) => return Err(error),
    };
    refresh_modified_time(&file)?;
    if let Err(error) = revision_directory.verify_bindings() {
        if created {
            let _ = unlink_regular(&revision_directory, &filename, &file);
        }
        return Err(error);
    }
    ensure_reservation_root(&root, &content_key, reservation)?;
    revision_directory.sync()?;
    Ok(StagedReportArtifact {
        revision_id: revision_id.clone(),
        format,
        staging_key: expected_staging_key(&revision_id, &filename),
        sha256,
        byte_len: u64::try_from(bytes.len()).context("report_artifact_too_large")?,
    })
}

pub(super) async fn stage(
    project_root: &Path,
    revision_id: &str,
    format: ReportArtifactFormat,
    bytes: &[u8],
) -> Result<StagedReportArtifact> {
    let revision_id = sanitize_report_path_component(revision_id)?;
    let sha256 = sha256_hex(bytes);
    let filename = artifact_filename(&sha256, format)?;
    let content_key = expected_content_key(&filename);
    let reservation = acquire_content_lock(project_root.to_path_buf(), content_key).await?;
    let project_root = project_root.to_path_buf();
    let bytes = bytes.to_vec();
    tokio::task::spawn_blocking(move || {
        stage_blocking(&project_root, revision_id, format, bytes, &reservation)
    })
    .await
    .context("report_artifact_stage_join_failed")?
}

fn refresh_modified_time(file: &File) -> Result<()> {
    let times = [
        libc::timespec {
            tv_sec: 0,
            tv_nsec: libc::UTIME_OMIT,
        },
        libc::timespec {
            tv_sec: 0,
            tv_nsec: libc::UTIME_NOW,
        },
    ];
    if unsafe { libc::futimens(file.as_raw_fd(), times.as_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error())
            .context("report_artifact_timestamp_refresh_failed");
    }
    file.sync_all()?;
    Ok(())
}

fn promote_blocking(
    project_root: &Path,
    staged: &StagedReportArtifact,
    reservation: &ContentKeyReservation,
) -> Result<StoredReportArtifact> {
    let filename = validate_staged(staged)?;
    let content_key = expected_content_key(&filename);
    let root = AnchoredDirectory::open_project_root(project_root)?;
    ensure_reservation_root(&root, &content_key, reservation)?;
    let staging = root
        .descendant(
            &[".golish", "reports", ".staging", &staged.revision_id],
            false,
        )?
        .ok_or_else(|| anyhow::anyhow!("report_staging_missing"))?;
    let blobs = root
        .descendant(&[".golish", "reports", "blobs", "sha256"], true)?
        .ok_or_else(|| anyhow::anyhow!("report_blob_path_invalid"))?;
    staging.verify_bindings()?;
    blobs.verify_bindings()?;
    let staged_file = open_regular(&staging, &filename, libc::O_RDONLY, 0)?
        .ok_or_else(|| anyhow::anyhow!("report_staging_missing"))?;
    let staged_bytes = read_all(&staged_file)?;
    if sha256_hex(&staged_bytes) != staged.sha256
        || u64::try_from(staged_bytes.len()).ok() != Some(staged.byte_len)
    {
        anyhow::bail!("report_staging_identity_conflict");
    }
    verify_named_file(&staging, &filename, &staged_file)?;

    let c_filename = c_component(&filename)?;
    let link_result = unsafe {
        libc::linkat(
            staging.file.as_raw_fd(),
            c_filename.as_ptr(),
            blobs.file.as_raw_fd(),
            c_filename.as_ptr(),
            0,
        )
    };
    if link_result != 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::AlreadyExists {
            return Err(error).context("report_artifact_promote_link_failed");
        }
    }
    let blob_file = open_regular(&blobs, &filename, libc::O_RDWR, 0)?
        .ok_or_else(|| anyhow::anyhow!("report_blob_identity_conflict"))?;
    let blob_bytes = read_all(&blob_file)?;
    if sha256_hex(&blob_bytes) != staged.sha256
        || u64::try_from(blob_bytes.len()).ok() != Some(staged.byte_len)
    {
        anyhow::bail!("report_blob_identity_conflict");
    }
    refresh_modified_time(&blob_file)?;
    ensure_reservation_root(&root, &content_key, reservation)?;
    staging.verify_bindings()?;
    blobs.verify_bindings()?;
    unlink_regular(&staging, &filename, &staged_file)?;
    blobs.sync()?;
    ensure_reservation_root(&root, &content_key, reservation)?;
    Ok(StoredReportArtifact {
        format: staged.format,
        content_key: content_key.clone(),
        storage_path: expected_storage_path(&content_key),
        sha256: staged.sha256.clone(),
        byte_len: staged.byte_len,
    })
}

pub(super) async fn promote(
    project_root: &Path,
    staged: &StagedReportArtifact,
) -> Result<ReservedReportArtifact> {
    let filename = validate_staged(staged)?;
    let content_key = expected_content_key(&filename);
    let reservation = acquire_content_lock(project_root.to_path_buf(), content_key).await?;
    let project_root = project_root.to_path_buf();
    let staged = staged.clone();
    let (artifact, reservation) = tokio::task::spawn_blocking(move || {
        let artifact = promote_blocking(&project_root, &staged, &reservation)?;
        Ok::<_, anyhow::Error>((artifact, reservation))
    })
    .await
    .context("report_artifact_promote_join_failed")??;
    Ok(ReservedReportArtifact::new(artifact, reservation))
}

fn verify_blocking(project_root: &Path, artifact: &StoredReportArtifact) -> Result<bool> {
    let filename = validate_stored(artifact)?;
    let root = AnchoredDirectory::open_project_root(project_root)?;
    let Some(blobs) = root.descendant(&[".golish", "reports", "blobs", "sha256"], false)? else {
        return Ok(false);
    };
    let Some(file) = open_regular(&blobs, &filename, libc::O_RDONLY, 0)? else {
        return Ok(false);
    };
    let bytes = read_all(&file)?;
    verify_named_file(&blobs, &filename, &file)?;
    blobs.verify_bindings()?;
    Ok(sha256_hex(&bytes) == artifact.sha256
        && u64::try_from(bytes.len()).ok() == Some(artifact.byte_len))
}

pub(super) async fn verify(project_root: &Path, artifact: &StoredReportArtifact) -> Result<bool> {
    let project_root = project_root.to_path_buf();
    let artifact = artifact.clone();
    tokio::task::spawn_blocking(move || verify_blocking(&project_root, &artifact))
        .await
        .context("report_artifact_verify_join_failed")?
}

fn discard_blocking(
    project_root: &Path,
    staged: &StagedReportArtifact,
    reservation: &ContentKeyReservation,
) -> Result<()> {
    let filename = validate_staged(staged)?;
    let content_key = expected_content_key(&filename);
    let root = AnchoredDirectory::open_project_root(project_root)?;
    ensure_reservation_root(&root, &content_key, reservation)?;
    let Some(staging) = root.descendant(
        &[".golish", "reports", ".staging", &staged.revision_id],
        false,
    )?
    else {
        return Ok(());
    };
    let Some(file) = open_regular(&staging, &filename, libc::O_RDONLY, 0)? else {
        return Ok(());
    };
    unlink_regular(&staging, &filename, &file)?;
    ensure_reservation_root(&root, &content_key, reservation)?;
    Ok(())
}

pub(super) async fn discard(project_root: &Path, staged: &StagedReportArtifact) -> Result<()> {
    let filename = validate_staged(staged)?;
    let content_key = expected_content_key(&filename);
    let reservation = acquire_content_lock(project_root.to_path_buf(), content_key).await?;
    let project_root = project_root.to_path_buf();
    let staged = staged.clone();
    tokio::task::spawn_blocking(move || discard_blocking(&project_root, &staged, &reservation))
        .await
        .context("report_artifact_discard_join_failed")?
}

struct DirectoryStream(*mut libc::DIR);

impl Drop for DirectoryStream {
    fn drop(&mut self) {
        unsafe {
            libc::closedir(self.0);
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
unsafe fn errno_location() -> *mut libc::c_int {
    libc::__errno_location()
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
unsafe fn errno_location() -> *mut libc::c_int {
    libc::__error()
}

fn directory_entries(directory: &AnchoredDirectory) -> Result<Vec<(String, EntryKind)>> {
    directory.verify_bindings()?;
    let duplicate = unsafe { libc::dup(directory.file.as_raw_fd()) };
    if duplicate < 0 {
        return Err(std::io::Error::last_os_error()).context("report_artifact_dir_dup_failed");
    }
    let raw_stream = unsafe { libc::fdopendir(duplicate) };
    if raw_stream.is_null() {
        unsafe {
            libc::close(duplicate);
        }
        return Err(std::io::Error::last_os_error()).context("report_artifact_dir_stream_failed");
    }
    let stream = DirectoryStream(raw_stream);
    let mut entries = Vec::new();
    loop {
        unsafe {
            *errno_location() = 0;
        }
        let entry = unsafe { libc::readdir(stream.0) };
        if entry.is_null() {
            let errno = unsafe { *errno_location() };
            if errno != 0 {
                return Err(std::io::Error::from_raw_os_error(errno))
                    .context("report_artifact_dir_read_failed");
            }
            break;
        }
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
        if name.to_bytes() == b"." || name.to_bytes() == b".." {
            continue;
        }
        let name = std::str::from_utf8(name.to_bytes())
            .context("report_artifact_filename_not_utf8")?
            .to_string();
        let stat = stat_at(&directory.file, &c_component(&name)?)?
            .ok_or_else(|| anyhow::anyhow!("report_artifact_path_binding_changed"))?;
        entries.push((name, kind_from_stat(&stat)));
    }
    directory.verify_bindings()?;
    Ok(entries)
}

#[derive(Clone)]
struct StagingCandidate {
    revision_id: String,
    filename: String,
}

fn enumerate_gc_candidates(project_root: &Path) -> Result<(Vec<StagingCandidate>, Vec<String>)> {
    let root = AnchoredDirectory::open_project_root(project_root)?;
    let mut staging_files = Vec::new();
    if let Some(staging) = root.descendant(&[".golish", "reports", ".staging"], false)? {
        for (revision_id, kind) in directory_entries(&staging)? {
            if kind == EntryKind::Symlink {
                anyhow::bail!("report_artifact_symlink_forbidden");
            }
            if kind != EntryKind::Directory {
                anyhow::bail!("report_artifact_path_not_directory");
            }
            sanitize_report_path_component(&revision_id)?;
            let revision = staging
                .child_directory(&revision_id, false)?
                .ok_or_else(|| anyhow::anyhow!("report_artifact_path_binding_changed"))?;
            for (filename, kind) in directory_entries(&revision)? {
                if kind == EntryKind::Symlink {
                    anyhow::bail!("report_artifact_symlink_forbidden");
                }
                if kind != EntryKind::Regular {
                    anyhow::bail!("report_artifact_path_not_file");
                }
                parse_artifact_filename(&filename)?;
                staging_files.push(StagingCandidate {
                    revision_id: revision_id.clone(),
                    filename,
                });
            }
        }
    }
    let mut blobs = Vec::new();
    if let Some(blob_directory) =
        root.descendant(&[".golish", "reports", "blobs", "sha256"], false)?
    {
        for (filename, kind) in directory_entries(&blob_directory)? {
            if kind == EntryKind::Symlink {
                anyhow::bail!("report_artifact_symlink_forbidden");
            }
            if kind != EntryKind::Regular {
                anyhow::bail!("report_artifact_path_not_file");
            }
            parse_artifact_filename(&filename)?;
            blobs.push(filename);
        }
    }
    Ok((staging_files, blobs))
}

fn is_expired(file: &File, now: SystemTime, grace_period: Duration) -> Result<bool> {
    let modified = file.metadata()?.modified()?;
    Ok(now.duration_since(modified).unwrap_or_default() >= grace_period)
}

fn gc_staging_candidate(
    project_root: &Path,
    candidate: &StagingCandidate,
    now: SystemTime,
    grace_period: Duration,
    reservation: &ContentKeyReservation,
) -> Result<bool> {
    let content_key = expected_content_key(&candidate.filename);
    let root = AnchoredDirectory::open_project_root(project_root)?;
    ensure_reservation_root(&root, &content_key, reservation)?;
    let Some(directory) = root.descendant(
        &[".golish", "reports", ".staging", &candidate.revision_id],
        false,
    )?
    else {
        return Ok(false);
    };
    let Some(file) = open_regular(&directory, &candidate.filename, libc::O_RDONLY, 0)? else {
        return Ok(false);
    };
    if !is_expired(&file, now, grace_period)? {
        return Ok(false);
    }
    let deleted = unlink_regular(&directory, &candidate.filename, &file)?;
    ensure_reservation_root(&root, &content_key, reservation)?;
    Ok(deleted)
}

fn gc_blob_candidate(
    project_root: &Path,
    filename: &str,
    now: SystemTime,
    grace_period: Duration,
    reservation: &ContentKeyReservation,
) -> Result<bool> {
    let content_key = expected_content_key(filename);
    let root = AnchoredDirectory::open_project_root(project_root)?;
    ensure_reservation_root(&root, &content_key, reservation)?;
    let Some(directory) = root.descendant(&[".golish", "reports", "blobs", "sha256"], false)?
    else {
        return Ok(false);
    };
    let Some(file) = open_regular(&directory, filename, libc::O_RDONLY, 0)? else {
        return Ok(false);
    };
    if !is_expired(&file, now, grace_period)? {
        return Ok(false);
    }
    let deleted = unlink_regular(&directory, filename, &file)?;
    ensure_reservation_root(&root, &content_key, reservation)?;
    Ok(deleted)
}

pub(super) async fn gc(
    project_root: &Path,
    now: SystemTime,
    grace_period: Duration,
    referenced_content_keys: &BTreeSet<String>,
) -> Result<ReportArtifactGcOutcome> {
    let project_root_buf = project_root.to_path_buf();
    let (staging, blobs) = tokio::task::spawn_blocking({
        let project_root = project_root_buf.clone();
        move || enumerate_gc_candidates(&project_root)
    })
    .await
    .context("report_artifact_gc_enumerate_join_failed")??;
    let mut outcome = ReportArtifactGcOutcome::default();
    for candidate in staging {
        let content_key = expected_content_key(&candidate.filename);
        let reservation = acquire_content_lock(project_root_buf.clone(), content_key).await?;
        let project_root = project_root_buf.clone();
        if tokio::task::spawn_blocking(move || {
            gc_staging_candidate(&project_root, &candidate, now, grace_period, &reservation)
        })
        .await
        .context("report_artifact_gc_staging_join_failed")??
        {
            outcome.deleted_staging += 1;
        }
    }
    for filename in blobs {
        let content_key = expected_content_key(&filename);
        if referenced_content_keys.contains(&content_key) {
            continue;
        }
        let reservation = acquire_content_lock(project_root_buf.clone(), content_key).await?;
        let project_root = project_root_buf.clone();
        if tokio::task::spawn_blocking(move || {
            gc_blob_candidate(&project_root, &filename, now, grace_period, &reservation)
        })
        .await
        .context("report_artifact_gc_blob_join_failed")??
        {
            outcome.deleted_blobs += 1;
        }
    }
    Ok(outcome)
}

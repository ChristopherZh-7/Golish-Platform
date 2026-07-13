//! Capability-based Windows report-artifact storage.
//!
//! `cap-std`/`cap-primitives` keep every operation relative to retained
//! directory handles. Project-root components are opened one at a time with
//! no-follow semantics, and every directory/file carrying the reparse-point
//! attribute (including junctions) is rejected.

use std::collections::{BTreeSet, HashMap};
use std::ffi::OsStr;
use std::fs::File as StdFile;
use std::io::{Read, Write};
use std::os::windows::fs::MetadataExt as StdMetadataExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use cap_primitives::fs::{FollowSymlinks, MetadataExt as CapMetadataExt, OpenOptionsExt as _};
use cap_std::fs::{Dir, File, OpenOptions};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use windows_sys::Win32::Foundation::GENERIC_READ;
use windows_sys::Win32::Storage::FileSystem::{
    FileDispositionInfo, GetFileInformationByHandle, SetFileInformationByHandle,
    BY_HANDLE_FILE_INFORMATION, DELETE, FILE_DISPOSITION_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE,
};

use super::crud_ops::{
    sanitize_report_path_component, sha256_hex, ReportArtifactFormat, ReportArtifactGcOutcome,
    ReservedReportArtifact, StagedReportArtifact, StoredReportArtifact,
};

const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileIdentity {
    volume: u32,
    index: u64,
}

#[cfg(test)]
type WindowsFsRaceAction = Box<dyn FnOnce() + Send + 'static>;

#[cfg(test)]
struct WindowsFsRaceHook {
    directory_identity: FileIdentity,
    filename: String,
    action: WindowsFsRaceAction,
}

#[cfg(test)]
type WindowsFsRaceHookSlot = OnceLock<Mutex<Option<WindowsFsRaceHook>>>;

#[cfg(test)]
static VERIFY_AFTER_HASH_HOOK: WindowsFsRaceHookSlot = OnceLock::new();

#[cfg(test)]
static DELETE_BEFORE_DISPOSITION_HOOK: WindowsFsRaceHookSlot = OnceLock::new();

#[cfg(test)]
static PROMOTE_BEFORE_STAGING_DELETE_HOOK: WindowsFsRaceHookSlot = OnceLock::new();

fn std_identity(file: &StdFile) -> Result<FileIdentity> {
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    let succeeded = unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) };
    if succeeded == 0 {
        return Err(std::io::Error::last_os_error())
            .context("report_artifact_file_identity_unavailable");
    }
    Ok(FileIdentity {
        volume: information.dwVolumeSerialNumber,
        index: (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow),
    })
}

fn cap_identity(file: &File) -> Result<FileIdentity> {
    std_identity(&file.try_clone()?.into_std())
}

fn ensure_not_reparse(attributes: u32) -> Result<()> {
    if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        anyhow::bail!("report_artifact_reparse_point_forbidden");
    }
    Ok(())
}

struct AnchoredDirectory {
    // Retaining every ancestor prevents rename/delete replacement on Windows;
    // cap-std opens directory handles without FILE_SHARE_DELETE.
    chain: Vec<Dir>,
}

impl AnchoredDirectory {
    fn open_project_root(path: &Path) -> Result<Self> {
        if !path.is_absolute() {
            anyhow::bail!("report_project_root_not_absolute");
        }
        let mut components = path.components();
        let prefix = match components.next() {
            Some(Component::Prefix(prefix)) => prefix,
            _ => anyhow::bail!("report_project_root_invalid"),
        };
        if !matches!(components.next(), Some(Component::RootDir)) {
            anyhow::bail!("report_project_root_invalid");
        }
        let mut volume_root = PathBuf::from(prefix.as_os_str());
        volume_root.push("\\");
        let volume = Dir::open_ambient_dir(&volume_root, cap_std::ambient_authority())
            .context("report_project_root_inaccessible")?;
        ensure_directory_handle(&volume)?;
        let mut anchored = Self {
            chain: vec![volume],
        };
        for component in components {
            let Component::Normal(component) = component else {
                anyhow::bail!("report_project_root_invalid");
            };
            anchored = anchored
                .child_os(component, false)?
                .ok_or_else(|| anyhow::anyhow!("report_project_root_inaccessible"))?;
        }
        Ok(anchored)
    }

    fn current(&self) -> &Dir {
        self.chain
            .last()
            .expect("anchored directory always has a volume root")
    }

    fn try_clone(&self) -> Result<Self> {
        Ok(Self {
            chain: self
                .chain
                .iter()
                .map(Dir::try_clone)
                .collect::<std::io::Result<_>>()?,
        })
    }

    fn identity(&self) -> Result<FileIdentity> {
        let file = self.current().try_clone()?.into_std_file();
        std_identity(&file)
    }

    fn child(&self, name: &str, create: bool) -> Result<Option<Self>> {
        validate_component(name)?;
        self.child_os(OsStr::new(name), create)
    }

    fn child_os(&self, name: &OsStr, create: bool) -> Result<Option<Self>> {
        if name.is_empty() || name == "." || name == ".." {
            anyhow::bail!("report_artifact_path_component_invalid");
        }
        let parent_file = self.current().try_clone()?.into_std_file();
        let opened = match cap_primitives::fs::open_dir_nofollow(&parent_file, Path::new(name)) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && !create => {
                return Ok(None)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && create => {
                match self.current().create_dir(name) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => {
                        return Err(error).context("report_artifact_directory_create_failed")
                    }
                }
                cap_primitives::fs::open_dir_nofollow(&parent_file, Path::new(name))
                    .context("report_artifact_directory_open_failed")?
            }
            Err(error) => {
                return Err(error).context("report_artifact_directory_open_failed");
            }
        };
        let child = Dir::from_std_file(opened);
        ensure_directory_handle(&child)?;
        let mut chain = self
            .chain
            .iter()
            .map(Dir::try_clone)
            .collect::<std::io::Result<Vec<_>>>()?;
        chain.push(child);
        Ok(Some(Self { chain }))
    }

    fn descendant(&self, components: &[&str], create: bool) -> Result<Option<Self>> {
        let mut current = self.try_clone()?;
        for component in components {
            let Some(child) = current.child(component, create)? else {
                return Ok(None);
            };
            current = child;
        }
        Ok(Some(current))
    }

    fn sync(&self) -> Result<()> {
        self.current().try_clone()?.into_std_file().sync_all()?;
        Ok(())
    }
}

#[cfg(test)]
fn install_windows_fs_race_hook(
    slot: &WindowsFsRaceHookSlot,
    directory: &AnchoredDirectory,
    filename: &str,
    action: WindowsFsRaceAction,
) {
    let hook = WindowsFsRaceHook {
        directory_identity: directory.identity().expect("race hook directory identity"),
        filename: filename.to_string(),
        action,
    };
    *slot
        .get_or_init(Default::default)
        .lock()
        .expect("Windows filesystem race hook mutex poisoned") = Some(hook);
}

#[cfg(test)]
fn run_windows_fs_race_hook(
    slot: &WindowsFsRaceHookSlot,
    directory: &AnchoredDirectory,
    filename: &str,
) {
    let Ok(directory_identity) = directory.identity() else {
        return;
    };
    let hook = {
        let mut guard = slot
            .get_or_init(Default::default)
            .lock()
            .expect("Windows filesystem race hook mutex poisoned");
        if guard.as_ref().is_some_and(|hook| {
            hook.directory_identity == directory_identity && hook.filename == filename
        }) {
            guard.take()
        } else {
            None
        }
    };
    if let Some(hook) = hook {
        (hook.action)();
    }
}

#[cfg(test)]
fn install_verify_after_hash_hook(
    directory: &AnchoredDirectory,
    filename: &str,
    action: WindowsFsRaceAction,
) {
    install_windows_fs_race_hook(&VERIFY_AFTER_HASH_HOOK, directory, filename, action);
}

#[cfg(test)]
fn run_verify_after_hash_hook(directory: &AnchoredDirectory, filename: &str) {
    run_windows_fs_race_hook(&VERIFY_AFTER_HASH_HOOK, directory, filename);
}

#[cfg(test)]
fn install_delete_before_disposition_hook(
    directory: &AnchoredDirectory,
    filename: &str,
    action: WindowsFsRaceAction,
) {
    install_windows_fs_race_hook(&DELETE_BEFORE_DISPOSITION_HOOK, directory, filename, action);
}

#[cfg(test)]
fn run_delete_before_disposition_hook(directory: &AnchoredDirectory, filename: &str) {
    run_windows_fs_race_hook(&DELETE_BEFORE_DISPOSITION_HOOK, directory, filename);
}

#[cfg(test)]
fn install_promote_before_staging_delete_hook(
    directory: &AnchoredDirectory,
    filename: &str,
    action: WindowsFsRaceAction,
) {
    install_windows_fs_race_hook(
        &PROMOTE_BEFORE_STAGING_DELETE_HOOK,
        directory,
        filename,
        action,
    );
}

#[cfg(test)]
fn run_promote_before_staging_delete_hook(directory: &AnchoredDirectory, filename: &str) {
    run_windows_fs_race_hook(&PROMOTE_BEFORE_STAGING_DELETE_HOOK, directory, filename);
}

fn ensure_directory_handle(directory: &Dir) -> Result<()> {
    let file = directory.try_clone()?.into_std_file();
    let metadata = file.metadata()?;
    ensure_not_reparse(metadata.file_attributes())?;
    if !metadata.is_dir() {
        anyhow::bail!("report_artifact_path_not_directory");
    }
    Ok(())
}

fn validate_component(name: &str) -> Result<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\'])
        || name.contains('\0')
    {
        anyhow::bail!("report_artifact_path_component_invalid");
    }
    Ok(())
}

fn nofollow(options: &mut OpenOptions) {
    options._cap_fs_ext_follow(FollowSymlinks::No);
    // cap-primitives defaults to READ|WRITE|DELETE sharing. Report artifact
    // handles deliberately deny FILE_SHARE_DELETE so a checked path cannot be
    // renamed or replaced while its exact handle is being hashed, linked,
    // locked, or marked for deletion.
    options.share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE);
}

fn open_regular_with_share_mode(
    directory: &AnchoredDirectory,
    name: &str,
    mut options: OpenOptions,
    share_mode: u32,
) -> Result<Option<File>> {
    validate_component(name)?;
    nofollow(&mut options);
    options.share_mode(share_mode);
    let file = match directory.current().open_with(name, &options) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("report_artifact_file_open_failed"),
    };
    let metadata = file.metadata()?;
    ensure_not_reparse(CapMetadataExt::file_attributes(&metadata))?;
    if !metadata.is_file() {
        anyhow::bail!("report_artifact_path_not_file");
    }
    Ok(Some(file))
}

fn open_regular(
    directory: &AnchoredDirectory,
    name: &str,
    options: OpenOptions,
) -> Result<Option<File>> {
    open_regular_with_share_mode(directory, name, options, FILE_SHARE_READ | FILE_SHARE_WRITE)
}

fn read_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true);
    options
}

fn read_write_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    options
}

fn create_new_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    options
}

fn delete_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).access_mode(GENERIC_READ | DELETE);
    options
}

fn read_all(mut file: &File) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn refresh_modified_time(file: &File) -> Result<()> {
    let std_file = file.try_clone()?.into_std();
    std_file.set_times(std::fs::FileTimes::new().set_modified(SystemTime::now()))?;
    std_file.sync_all()?;
    Ok(())
}

fn verify_named_identity(
    directory: &AnchoredDirectory,
    name: &str,
    expected: FileIdentity,
) -> Result<()> {
    // This short-lived identity probe must share DELETE when the retained
    // handle itself requests DELETE access. The retained ordinary/delete
    // handle still denies share-delete, so the name cannot move during the
    // comparison.
    let named = open_regular_with_share_mode(
        directory,
        name,
        read_options(),
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
    )?
    .ok_or_else(|| anyhow::anyhow!("report_artifact_named_file_missing"))?;
    if cap_identity(&named)? != expected {
        anyhow::bail!("report_artifact_named_file_identity_changed");
    }
    Ok(())
}

fn verify_named_file(directory: &AnchoredDirectory, name: &str, file: &File) -> Result<()> {
    verify_named_identity(directory, name, cap_identity(file)?)
}

fn delete_regular_by_handle(directory: &AnchoredDirectory, name: &str, file: File) -> Result<bool> {
    let expected = cap_identity(&file)?;
    verify_named_identity(directory, name, expected)?;
    #[cfg(test)]
    run_delete_before_disposition_hook(directory, name);
    verify_named_identity(directory, name, expected)?;

    let file = file.into_std();
    if std_identity(&file)? != expected {
        anyhow::bail!("report_artifact_delete_handle_identity_changed");
    }
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    let succeeded = unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileDispositionInfo,
            std::ptr::from_ref(&disposition).cast(),
            u32::try_from(std::mem::size_of::<FILE_DISPOSITION_INFO>())
                .expect("FILE_DISPOSITION_INFO size fits u32"),
        )
    };
    if succeeded == 0 {
        return Err(std::io::Error::last_os_error())
            .context("report_artifact_handle_delete_failed");
    }
    drop(file);
    directory.sync()?;
    Ok(true)
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
    file: StdFile,
    file_identity: FileIdentity,
    content_key: String,
    root_identity: FileIdentity,
    _in_process: OwnedMutexGuard<()>,
}

impl ContentKeyReservation {
    fn verify(&self) -> Result<()> {
        ensure_directory_handle(self.directory.current())?;
        if std_identity(&self.file)? != self.file_identity {
            anyhow::bail!("report_artifact_lock_handle_identity_changed");
        }
        verify_named_identity(&self.directory, &self.filename, self.file_identity)
            .context("report_artifact_lock_name_identity_changed")
    }
}

impl Drop for ContentKeyReservation {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.file);
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
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    let file = open_regular(&prepared.directory, &prepared.filename, options)?
        .ok_or_else(|| anyhow::anyhow!("report_artifact_lock_open_failed"))?
        .into_std();
    fs2::FileExt::lock_exclusive(&file).context("report_artifact_lock_failed")?;
    let file_identity = std_identity(&file)?;
    let reservation = ContentKeyReservation {
        directory: prepared.directory,
        filename: prepared.filename,
        file,
        file_identity,
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
    let process_key = format!("{:?}:{}", prepared.root_identity, prepared.content_key);
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
    let directory = root
        .descendant(&[".golish", "reports", ".staging", &revision_id], true)?
        .ok_or_else(|| anyhow::anyhow!("report_staging_path_invalid"))?;
    let file = match open_regular(&directory, &filename, create_new_options()) {
        Ok(Some(mut file)) => {
            file.write_all(&bytes)?;
            file.sync_all()?;
            file
        }
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::AlreadyExists) =>
        {
            let existing = open_regular(&directory, &filename, read_write_options())?
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
    verify_named_file(&directory, &filename, &file)?;
    directory.sync()?;
    ensure_reservation_root(&root, &content_key, reservation)?;
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
    let filename = artifact_filename(&sha256_hex(bytes), format)?;
    let reservation =
        acquire_content_lock(project_root.to_path_buf(), expected_content_key(&filename)).await?;
    let project_root = project_root.to_path_buf();
    let bytes = bytes.to_vec();
    tokio::task::spawn_blocking(move || {
        stage_blocking(&project_root, revision_id, format, bytes, &reservation)
    })
    .await
    .context("report_artifact_stage_join_failed")?
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
    let staged_file = open_regular(&staging, &filename, read_options())?
        .ok_or_else(|| anyhow::anyhow!("report_staging_missing"))?;
    let staged_bytes = read_all(&staged_file)?;
    if sha256_hex(&staged_bytes) != staged.sha256
        || u64::try_from(staged_bytes.len()).ok() != Some(staged.byte_len)
    {
        anyhow::bail!("report_staging_identity_conflict");
    }
    let staged_identity = cap_identity(&staged_file)?;
    verify_named_identity(&staging, &filename, staged_identity)?;
    match staging
        .current()
        .hard_link(&filename, blobs.current(), &filename)
    {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error).context("report_artifact_promote_link_failed"),
    }
    let blob_file = open_regular(&blobs, &filename, read_write_options())?
        .ok_or_else(|| anyhow::anyhow!("report_blob_identity_conflict"))?;
    let blob_identity = cap_identity(&blob_file)?;
    let blob_bytes = read_all(&blob_file)?;
    if sha256_hex(&blob_bytes) != staged.sha256
        || u64::try_from(blob_bytes.len()).ok() != Some(staged.byte_len)
    {
        anyhow::bail!("report_blob_identity_conflict");
    }
    verify_named_file(&blobs, &filename, &blob_file)?;
    refresh_modified_time(&blob_file)?;
    verify_named_file(&blobs, &filename, &blob_file)?;
    ensure_reservation_root(&root, &content_key, reservation)?;
    drop(blob_file);
    drop(staged_file);
    #[cfg(test)]
    run_promote_before_staging_delete_hook(&blobs, &filename);
    ensure_reservation_root(&root, &content_key, reservation)?;
    let staged_delete = open_regular(&staging, &filename, delete_options())?
        .ok_or_else(|| anyhow::anyhow!("report_staging_missing"))?;
    if cap_identity(&staged_delete)? != staged_identity {
        anyhow::bail!("report_staging_identity_conflict");
    }
    delete_regular_by_handle(&staging, &filename, staged_delete)?;
    blobs.sync()?;
    ensure_reservation_root(&root, &content_key, reservation)?;
    let final_blob =
        open_regular_with_share_mode(&blobs, &filename, read_options(), FILE_SHARE_READ)?
            .ok_or_else(|| anyhow::anyhow!("report_blob_identity_conflict"))?;
    if cap_identity(&final_blob)? != blob_identity {
        anyhow::bail!("report_blob_identity_conflict");
    }
    let final_blob_bytes = read_all(&final_blob)?;
    if sha256_hex(&final_blob_bytes) != staged.sha256
        || u64::try_from(final_blob_bytes.len()).ok() != Some(staged.byte_len)
    {
        anyhow::bail!("report_blob_identity_conflict");
    }
    verify_named_identity(&blobs, &filename, blob_identity)?;
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
    let reservation =
        acquire_content_lock(project_root.to_path_buf(), expected_content_key(&filename)).await?;
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
    let Some(file) = open_regular(&blobs, &filename, read_options())? else {
        return Ok(false);
    };
    let bytes = read_all(&file)?;
    #[cfg(test)]
    run_verify_after_hash_hook(&blobs, &filename);
    verify_named_file(&blobs, &filename, &file)?;
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
    if let Some(file) = open_regular(&staging, &filename, delete_options())? {
        delete_regular_by_handle(&staging, &filename, file)?;
    }
    ensure_reservation_root(&root, &content_key, reservation)?;
    Ok(())
}

pub(super) async fn discard(project_root: &Path, staged: &StagedReportArtifact) -> Result<()> {
    let filename = validate_staged(staged)?;
    let reservation =
        acquire_content_lock(project_root.to_path_buf(), expected_content_key(&filename)).await?;
    let project_root = project_root.to_path_buf();
    let staged = staged.clone();
    tokio::task::spawn_blocking(move || discard_blocking(&project_root, &staged, &reservation))
        .await
        .context("report_artifact_discard_join_failed")?
}

fn directory_entries(directory: &AnchoredDirectory) -> Result<Vec<(String, bool)>> {
    let mut result = Vec::new();
    for entry in directory.current().entries()? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("report_artifact_filename_not_utf8"))?;
        let metadata = directory.current().symlink_metadata(&name)?;
        ensure_not_reparse(CapMetadataExt::file_attributes(&metadata))?;
        result.push((name, metadata.is_dir()));
    }
    Ok(result)
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
        for (revision_id, is_dir) in directory_entries(&staging)? {
            if !is_dir {
                anyhow::bail!("report_artifact_path_not_directory");
            }
            sanitize_report_path_component(&revision_id)?;
            let revision = staging
                .child(&revision_id, false)?
                .ok_or_else(|| anyhow::anyhow!("report_artifact_path_binding_changed"))?;
            for (filename, is_dir) in directory_entries(&revision)? {
                if is_dir {
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
    if let Some(directory) = root.descendant(&[".golish", "reports", "blobs", "sha256"], false)? {
        for (filename, is_dir) in directory_entries(&directory)? {
            if is_dir {
                anyhow::bail!("report_artifact_path_not_file");
            }
            parse_artifact_filename(&filename)?;
            blobs.push(filename);
        }
    }
    Ok((staging_files, blobs))
}

fn is_expired(file: &File, now: SystemTime, grace_period: Duration) -> Result<bool> {
    let modified = file.metadata()?.modified()?.into_std();
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
    let Some(file) = open_regular(&directory, &candidate.filename, delete_options())? else {
        return Ok(false);
    };
    if !is_expired(&file, now, grace_period)? {
        return Ok(false);
    }
    let deleted = delete_regular_by_handle(&directory, &candidate.filename, file)?;
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
    let Some(file) = open_regular(&directory, filename, delete_options())? else {
        return Ok(false);
    };
    if !is_expired(&file, now, grace_period)? {
        return Ok(false);
    }
    let deleted = delete_regular_by_handle(&directory, filename, file)?;
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
        let reservation = acquire_content_lock(
            project_root_buf.clone(),
            expected_content_key(&candidate.filename),
        )
        .await?;
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

#[cfg(test)]
mod tests {
    use std::os::windows::fs::OpenOptionsExt as _;
    use std::sync::{Arc, Mutex};

    use windows_sys::Win32::Storage::FileSystem::{
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    use super::*;

    fn canonical_temp_root() -> (tempfile::TempDir, PathBuf) {
        let root = tempfile::tempdir().expect("temporary Windows report root");
        let canonical = std::fs::canonicalize(root.path()).expect("canonical report root");
        (root, canonical)
    }

    fn blob_directory(root: &Path) -> AnchoredDirectory {
        AnchoredDirectory::open_project_root(root)
            .expect("open report project root")
            .descendant(&[".golish", "reports", "blobs", "sha256"], true)
            .expect("open blob descendants")
            .expect("blob directory exists")
    }

    #[test]
    fn regular_file_handle_blocks_rename_and_replacement() {
        let (_temp, root) = canonical_temp_root();
        let blobs = blob_directory(&root);
        let filename = format!("{}.md", "1".repeat(64));
        let path = root.join(".golish/reports/blobs/sha256").join(&filename);
        let detached = path.with_extension("detached");
        let replacement = path.with_extension("replacement");
        let mut file = open_regular(&blobs, &filename, create_new_options())
            .expect("open regular blob")
            .expect("created regular blob");
        file.write_all(b"original").expect("write original blob");
        file.sync_all().expect("sync original blob");
        std::fs::write(&replacement, b"replacement").expect("write replacement candidate");

        assert!(
            std::fs::rename(&path, &detached).is_err(),
            "a retained artifact handle must deny rename/delete replacement"
        );
        assert!(
            std::fs::rename(&replacement, &path).is_err(),
            "a retained artifact handle must deny replacement by another file"
        );
        verify_named_file(&blobs, &filename, &file).expect("name remains bound to open handle");
        assert_eq!(std::fs::read(&path).expect("read named blob"), b"original");
        assert!(!detached.exists());
        assert_eq!(
            std::fs::read(&replacement).expect("read rejected replacement"),
            b"replacement"
        );
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn content_lock_handle_blocks_inode_split() {
        let (_temp, root) = canonical_temp_root();
        let filename = format!("{}.md", "2".repeat(64));
        let content_key = expected_content_key(&filename);
        let reservation = acquire_content_lock(root.clone(), content_key.clone())
            .await
            .expect("acquire content lock");
        let lock_path = root
            .join(".golish/reports/.locks/sha256")
            .join(format!("{filename}.lock"));
        let detached = lock_path.with_extension("detached");

        assert!(
            std::fs::rename(&lock_path, &detached).is_err(),
            "the named lock inode must not be replaceable while reserved"
        );
        reservation
            .verify()
            .expect("reservation keeps exact name-to-handle identity");
        assert!(lock_path.exists());
        assert!(!detached.exists());
        drop(reservation);

        // Characterize the identity validator independently of the production
        // no-delete-share protection: a deliberately permissive retained lock
        // handle can be split from its name, and the reservation must reject it.
        let prepared = prepare_content_lock(&root, &content_key).expect("prepare permissive lock");
        let process_key = format!("{:?}:{}", prepared.root_identity, prepared.content_key);
        let in_process = in_process_lock(&process_key).lock_owned().await;
        let permissive_file = open_regular_with_share_mode(
            &prepared.directory,
            &prepared.filename,
            read_write_options(),
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        )
        .expect("open permissive lock")
        .expect("permissive lock exists")
        .into_std();
        fs2::FileExt::lock_exclusive(&permissive_file).expect("lock permissive handle");
        let file_identity = std_identity(&permissive_file).expect("permissive lock identity");
        let permissive_reservation = ContentKeyReservation {
            directory: prepared.directory,
            filename: prepared.filename,
            file: permissive_file,
            file_identity,
            content_key: prepared.content_key,
            root_identity: prepared.root_identity,
            _in_process: in_process,
        };
        std::fs::rename(&lock_path, &detached).expect("split permissive lock inode");
        std::fs::write(&lock_path, b"replacement lock").expect("install replacement lock inode");
        assert!(
            permissive_reservation.verify().is_err(),
            "reservation identity check must reject a name bound to another lock inode"
        );
    }

    #[test]
    #[serial_test::serial]
    fn verify_hash_then_rechecks_the_named_blob_identity() {
        let (_temp, root) = canonical_temp_root();
        let bytes = b"verified report bytes";
        let sha256 = sha256_hex(bytes);
        let filename = format!("{sha256}.md");
        let blobs = blob_directory(&root);
        let path = root.join(".golish/reports/blobs/sha256").join(&filename);
        let detached = path.with_extension("detached");
        let mut file = open_regular(&blobs, &filename, create_new_options())
            .expect("open verified blob")
            .expect("create verified blob");
        file.write_all(bytes).expect("write verified blob");
        file.sync_all().expect("sync verified blob");
        drop(file);

        let swap_was_blocked = Arc::new(Mutex::new(false));
        let hook_result = Arc::clone(&swap_was_blocked);
        let hook_path = path.clone();
        let hook_detached = detached.clone();
        install_verify_after_hash_hook(
            &blobs,
            &filename,
            Box::new(move || {
                let rename = std::fs::rename(&hook_path, &hook_detached);
                *hook_result.lock().expect("verify hook result mutex") = rename.is_err();
                if rename.is_ok() {
                    std::fs::write(&hook_path, b"replacement").expect("write replacement blob");
                }
            }),
        );
        let artifact = StoredReportArtifact {
            format: ReportArtifactFormat::Markdown,
            content_key: expected_content_key(&filename),
            storage_path: expected_storage_path(&expected_content_key(&filename)),
            sha256,
            byte_len: u64::try_from(bytes.len()).expect("byte length"),
        };
        assert!(verify_blocking(&root, &artifact).expect("verify exact named blob"));
        assert!(
            *swap_was_blocked.lock().expect("read verify hook result"),
            "the hash handle must remain name-bound through the final identity check"
        );

        // Independently prove that the identity check rejects a detached old
        // handle even if an intentionally permissive caller allows replacement.
        let permissive = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .open(&path)
            .expect("open permissive verification handle");
        let expected = std_identity(&permissive).expect("old blob identity");
        std::fs::rename(&path, &detached).expect("detach permissive old blob");
        std::fs::write(&path, b"replacement").expect("install replacement blob");
        assert!(
            verify_named_identity(&blobs, &filename, expected).is_err(),
            "a replacement name must not attest the detached hash handle"
        );
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn gc_uses_verified_handle_delete_without_name_swap() {
        let (_temp, root) = canonical_temp_root();
        let staged = stage(
            &root,
            "019f0000-0000-7000-8000-000000000099",
            ReportArtifactFormat::Json,
            br#"{"orphan":true}"#,
        )
        .await
        .expect("stage orphan blob");
        let reserved = promote(&root, &staged).await.expect("promote orphan blob");
        let filename = artifact_filename(&staged.sha256, staged.format).expect("artifact filename");
        let blobs = blob_directory(&root);
        let blob_path = root.join(&reserved.storage_path);
        let detached = blob_path.with_extension("detached");
        drop(reserved);

        let swap_was_blocked = Arc::new(Mutex::new(false));
        let hook_result = Arc::clone(&swap_was_blocked);
        let hook_path = blob_path.clone();
        let hook_detached = detached.clone();
        install_delete_before_disposition_hook(
            &blobs,
            &filename,
            Box::new(move || {
                let rename = std::fs::rename(&hook_path, &hook_detached);
                *hook_result.lock().expect("delete hook result mutex") = rename.is_err();
                if rename.is_ok() {
                    std::fs::write(&hook_path, b"replacement must survive")
                        .expect("write replacement blob");
                }
            }),
        );

        let outcome = gc(
            &root,
            SystemTime::now() + Duration::from_secs(1),
            Duration::ZERO,
            &BTreeSet::new(),
        )
        .await
        .expect("GC exact orphan handle");
        assert_eq!(outcome.deleted_blobs, 1);
        assert!(
            *swap_was_blocked.lock().expect("read delete hook result"),
            "GC must keep the verified file name non-replaceable until disposition is set"
        );
        assert!(!blob_path.exists());
        assert!(!detached.exists());
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn promote_rejects_blob_swap_in_delete_reopen_gap() {
        let (_temp, root) = canonical_temp_root();
        let bytes = br#"{"final":"trusted"}"#;
        let staged = stage(
            &root,
            "019f0000-0000-7000-8000-000000000100",
            ReportArtifactFormat::Json,
            bytes,
        )
        .await
        .expect("stage trusted blob");
        let filename = artifact_filename(&staged.sha256, staged.format).expect("artifact filename");
        let blobs = blob_directory(&root);
        let blob_path = root.join(".golish/reports/blobs/sha256").join(&filename);
        let detached = blob_path.with_extension("detached");
        let swap_succeeded = Arc::new(Mutex::new(false));
        let hook_result = Arc::clone(&swap_succeeded);
        let hook_path = blob_path.clone();
        let hook_detached = detached.clone();
        install_promote_before_staging_delete_hook(
            &blobs,
            &filename,
            Box::new(move || {
                std::fs::rename(&hook_path, &hook_detached).expect("detach verified blob");
                std::fs::write(&hook_path, b"replacement").expect("install replacement blob");
                *hook_result.lock().expect("promotion hook result mutex") = true;
            }),
        );

        let promoted = promote(&root, &staged).await;
        assert!(
            *swap_succeeded.lock().expect("read promotion hook result"),
            "the regression hook must exercise the former drop/reopen gap"
        );
        assert!(
            promoted.is_err(),
            "promotion must not attest a replacement installed after blob verification"
        );
        assert_eq!(
            std::fs::read(&detached).expect("read detached trusted blob"),
            bytes
        );
        assert_eq!(
            std::fs::read(&blob_path).expect("read rejected replacement"),
            b"replacement"
        );
    }
}

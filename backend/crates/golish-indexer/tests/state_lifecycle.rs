//! Integration tests for `IndexerState` lifecycle and pure path helpers.
//!
//! These tests exercise the public surface of `golish-indexer` without
//! touching `vtcode-indexer` or any real on-disk state outside of
//! tempdirs. The vtcode bridge is verified separately at the application
//! layer where a real workspace is available.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use golish_indexer::{
    compute_index_dir, contract_home_dir, expand_home_dir, find_existing_index_dir,
    get_codebase_file_count, CodeSearchResult, IndexerBackend, IndexerState,
};
use golish_settings::schema::IndexLocation;
use tempfile::TempDir;

#[derive(Default)]
struct CountingBackend {
    files_indexed: Arc<AtomicUsize>,
    dirs_indexed: Arc<AtomicUsize>,
    search_calls: Arc<AtomicUsize>,
    find_calls: Arc<AtomicUsize>,
}

impl IndexerBackend for CountingBackend {
    fn index_file(&mut self, _path: &Path) -> anyhow::Result<()> {
        self.files_indexed.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn index_directory(&mut self, _path: &Path) -> anyhow::Result<()> {
        self.dirs_indexed.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn all_files(&self) -> Vec<String> {
        vec!["a.rs".into(), "b.rs".into()]
    }
    fn search(
        &self,
        pattern: &str,
        _path_filter: Option<&str>,
    ) -> anyhow::Result<Vec<CodeSearchResult>> {
        self.search_calls.fetch_add(1, Ordering::SeqCst);
        Ok(vec![CodeSearchResult {
            file_path: "a.rs".into(),
            line_number: 1,
            line_content: format!("// hit: {}", pattern),
            matches: vec![pattern.to_string()],
        }])
    }
    fn find_files(&self, pattern: &str) -> anyhow::Result<Vec<String>> {
        self.find_calls.fetch_add(1, Ordering::SeqCst);
        Ok(vec![format!("matched-{}.rs", pattern)])
    }
}

#[test]
fn lifecycle_uninit_then_set_then_shutdown() {
    let state = IndexerState::new();
    assert!(!state.is_initialized());
    assert!(state.workspace_root().is_none());
    assert!(state.with_indexer(|_| Ok(())).is_err(),
        "with_indexer must error before set_backend");

    let workspace = PathBuf::from("/tmp/golish-test-ws");
    state.set_backend(Box::<CountingBackend>::default(), workspace.clone());
    assert!(state.is_initialized());
    assert_eq!(state.workspace_root(), Some(workspace));

    state.shutdown();
    assert!(!state.is_initialized());
    assert!(state.workspace_root().is_none());
    assert!(state.with_indexer(|_| Ok(())).is_err(),
        "with_indexer must error after shutdown");
}

#[test]
fn with_indexer_dispatches_reads_to_backend() {
    let state = IndexerState::new();
    let backend = CountingBackend::default();
    let search_calls = backend.search_calls.clone();
    let find_calls = backend.find_calls.clone();
    state.set_backend(Box::new(backend), PathBuf::from("/tmp/ws"));

    let hits = state
        .with_indexer(|b| b.search("fn ", None))
        .expect("search ok");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].matches, vec!["fn ".to_string()]);

    let files = state
        .with_indexer(|b| b.find_files("foo"))
        .expect("find_files ok");
    assert_eq!(files, vec!["matched-foo.rs"]);
    let all = state.with_indexer(|b| Ok(b.all_files())).unwrap();
    assert_eq!(all, vec!["a.rs", "b.rs"]);

    assert_eq!(search_calls.load(Ordering::SeqCst), 1);
    assert_eq!(find_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn with_indexer_mut_dispatches_writes_to_backend() {
    let state = IndexerState::new();
    let backend = CountingBackend::default();
    let files = backend.files_indexed.clone();
    let dirs = backend.dirs_indexed.clone();
    state.set_backend(Box::new(backend), PathBuf::from("/tmp/ws"));

    state
        .with_indexer_mut(|b| b.index_file(Path::new("/x/a.rs")))
        .unwrap();
    state
        .with_indexer_mut(|b| b.index_file(Path::new("/x/b.rs")))
        .unwrap();
    state
        .with_indexer_mut(|b| b.index_directory(Path::new("/x")))
        .unwrap();

    assert_eq!(files.load(Ordering::SeqCst), 2);
    assert_eq!(dirs.load(Ordering::SeqCst), 1);
}

#[test]
fn second_set_backend_replaces_first() {
    let state = IndexerState::new();

    let first = CountingBackend::default();
    let first_files = first.files_indexed.clone();
    state.set_backend(Box::new(first), PathBuf::from("/tmp/first"));
    state
        .with_indexer_mut(|b| b.index_file(Path::new("/a")))
        .unwrap();
    assert_eq!(first_files.load(Ordering::SeqCst), 1);

    let second = CountingBackend::default();
    let second_files = second.files_indexed.clone();
    state.set_backend(Box::new(second), PathBuf::from("/tmp/second"));
    assert_eq!(state.workspace_root(), Some(PathBuf::from("/tmp/second")));

    state
        .with_indexer_mut(|b| b.index_file(Path::new("/b")))
        .unwrap();
    assert_eq!(
        first_files.load(Ordering::SeqCst),
        1,
        "first backend must not be touched after replacement"
    );
    assert_eq!(second_files.load(Ordering::SeqCst), 1);
}

#[test]
fn local_index_dir_is_under_workspace_dot_golish() {
    let workspace = PathBuf::from("/home/user/projects/sample");
    let dir = compute_index_dir(&workspace, IndexLocation::Local);
    assert_eq!(dir, workspace.join(".golish").join("index"));
}

#[test]
fn global_index_dir_is_under_home_codebases() {
    let workspace = PathBuf::from("/home/user/projects/sample");
    let dir = compute_index_dir(&workspace, IndexLocation::Global);
    let home = dirs::home_dir().expect("home dir");
    assert!(dir.starts_with(home.join(".golish").join("codebases")));
    assert!(dir.ends_with("index"));
}

#[test]
fn find_existing_index_dir_returns_none_for_unindexed_workspace() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().to_path_buf();
    assert!(find_existing_index_dir(&workspace, IndexLocation::Local).is_none());
}

#[test]
fn find_existing_index_dir_finds_local_layout() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().to_path_buf();
    let local_idx = workspace.join(".golish").join("index");
    std::fs::create_dir_all(&local_idx).unwrap();

    let found = find_existing_index_dir(&workspace, IndexLocation::Local)
        .expect("should detect local index dir");
    assert_eq!(found, local_idx);
}

#[test]
fn get_codebase_file_count_zero_when_no_index() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().to_path_buf();
    assert_eq!(get_codebase_file_count(&workspace), 0);
}

#[test]
fn get_codebase_file_count_counts_only_md_entries() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().to_path_buf();
    let local_idx = workspace.join(".golish").join("index");
    std::fs::create_dir_all(&local_idx).unwrap();

    std::fs::write(local_idx.join("a.md"), "x").unwrap();
    std::fs::write(local_idx.join("b.md"), "x").unwrap();
    std::fs::write(local_idx.join("c.json"), "ignored").unwrap();
    std::fs::write(local_idx.join("readme.txt"), "ignored").unwrap();

    assert_eq!(get_codebase_file_count(&workspace), 2);
}

#[test]
fn expand_home_dir_resolves_tilde_prefix() {
    let home = dirs::home_dir().expect("home dir");
    let expanded = expand_home_dir("~/foo/bar");
    assert_eq!(expanded, home.join("foo/bar"));
}

#[test]
fn expand_home_dir_leaves_absolute_path_untouched() {
    let p = expand_home_dir("/var/tmp/x");
    assert_eq!(p, PathBuf::from("/var/tmp/x"));
}

#[test]
fn contract_home_dir_round_trips_with_expand() {
    let home = dirs::home_dir().expect("home dir");
    let nested = home.join("projects").join("demo");
    let contracted = contract_home_dir(&nested);
    assert!(contracted.starts_with("~/"), "expected leading ~/, got {contracted}");
    assert_eq!(expand_home_dir(&contracted), nested);
}

#[test]
fn contract_home_dir_returns_input_when_not_under_home() {
    let outside = Path::new("/var/log/golish.log");
    let result = contract_home_dir(outside);
    assert_eq!(result, outside.to_string_lossy());
}

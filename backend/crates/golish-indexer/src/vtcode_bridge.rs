//! Bridge between `vtcode-indexer` and the [`IndexerBackend`] trait, so
//! upstream consumers never depend on `vtcode-indexer` directly.

use std::path::{Path, PathBuf};

use golish_settings::schema::IndexLocation;
use vtcode_indexer::SimpleIndexer;

use crate::paths::{compute_index_dir, find_existing_index_dir};
use crate::state::{CodeSearchResult, IndexerBackend, IndexerState};

pub struct VtcodeIndexerBackend {
    indexer: SimpleIndexer,
}

impl IndexerBackend for VtcodeIndexerBackend {
    fn index_file(&mut self, path: &Path) -> anyhow::Result<()> {
        self.indexer.index_file(path)?;
        Ok(())
    }

    fn index_directory(&mut self, path: &Path) -> anyhow::Result<()> {
        self.indexer.index_directory(path)?;
        Ok(())
    }

    fn all_files(&self) -> Vec<String> {
        self.indexer.all_files()
    }

    fn search(
        &self,
        pattern: &str,
        path_filter: Option<&str>,
    ) -> anyhow::Result<Vec<CodeSearchResult>> {
        let results = self.indexer.search(pattern, path_filter)?;
        Ok(results
            .into_iter()
            .map(|r| CodeSearchResult {
                file_path: r.file_path,
                line_number: r.line_number,
                line_content: r.line_content,
                matches: r.matches,
            })
            .collect())
    }

    fn find_files(&self, pattern: &str) -> anyhow::Result<Vec<String>> {
        let results = self.indexer.find_files(pattern)?;
        Ok(results)
    }
}

/// Initialize the indexer state with a vtcode-indexer backend.
///
/// This replaces the former `IndexerState::initialize_with_location` that
/// directly depended on vtcode-indexer.
pub fn initialize_vtcode_indexer(
    state: &IndexerState,
    workspace_path: PathBuf,
    index_location: IndexLocation,
) -> anyhow::Result<()> {
    tracing::info!(
        "Initializing vtcode indexer for workspace: {:?} with location: {:?}",
        workspace_path,
        index_location
    );

    let index_dir =
        if let Some(existing_dir) = find_existing_index_dir(&workspace_path, index_location) {
            tracing::info!("Found existing index at: {:?}", existing_dir);
            existing_dir
        } else {
            let new_dir = compute_index_dir(&workspace_path, index_location);
            tracing::debug!("Creating index directory: {:?}", new_dir);
            std::fs::create_dir_all(&new_dir)?;
            new_dir
        };

    let mut indexer = SimpleIndexer::with_index_dir(workspace_path.clone(), index_dir.clone());
    indexer.init()?;

    let loaded = load_existing_index(&mut indexer, &index_dir).unwrap_or(0);
    if loaded > 0 {
        tracing::info!("Loaded {} files from existing index", loaded);
    }

    state.set_backend(Box::new(VtcodeIndexerBackend { indexer }), workspace_path.clone());

    tracing::info!("Indexer initialized successfully for {:?}", workspace_path);
    tracing::info!("Index files will be stored in: {:?}", index_dir);
    Ok(())
}

fn load_existing_index(indexer: &mut SimpleIndexer, index_dir: &PathBuf) -> anyhow::Result<usize> {
    let mut loaded = 0;

    if !index_dir.exists() {
        return Ok(0);
    }

    for entry in std::fs::read_dir(index_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }

        if let Ok(content) = std::fs::read_to_string(&path) {
            for line in content.lines() {
                if let Some(file_path) = line.strip_prefix("- **Path**: ") {
                    let file_path = PathBuf::from(file_path.trim());
                    if file_path.exists() && indexer.index_file(&file_path).is_ok() {
                        loaded += 1;
                    }
                    break;
                }
            }
        }
    }

    Ok(loaded)
}

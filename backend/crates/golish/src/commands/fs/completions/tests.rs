//! Path completion tests.

use super::compute::{expand_tilde, parse_path_input};
use super::*;
use std::fs::{self, File};
use tempfile::TempDir;

/// Helper to create a test directory structure.
fn setup_test_dir() -> TempDir {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // Create directories
    fs::create_dir(root.join("Documents")).unwrap();
    fs::create_dir(root.join("Downloads")).unwrap();
    fs::create_dir(root.join("Desktop")).unwrap();
    fs::create_dir(root.join(".hidden_dir")).unwrap();

    // Create files
    File::create(root.join("file.txt")).unwrap();
    File::create(root.join("data.json")).unwrap();
    File::create(root.join(".hidden_file")).unwrap();

    // Create nested structure
    fs::create_dir_all(root.join("Documents/work")).unwrap();
    File::create(root.join("Documents/notes.md")).unwrap();

    temp
}

mod path_parsing;

mod filtering;

mod sorting;

mod entry_types;

mod insert_text;

mod limits;

mod edge_cases;

mod fuzzy_matching;

/// Property-based tests for path completion invariants.
mod property_tests;

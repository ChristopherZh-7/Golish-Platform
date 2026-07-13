#[test]
fn trusted_authorization_context_is_not_constructible_outside_memory_app() {
    let deps = std::env::current_exe()
        .expect("current test executable")
        .parent()
        .expect("deps directory")
        .to_path_buf();
    let memory_app = std::fs::read_dir(&deps)
        .expect("read deps directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with("libgolish_memory_app-") && name.ends_with(".rlib")
                })
        })
        .max_by_key(|path| {
            std::fs::metadata(path)
                .and_then(|metadata| metadata.modified())
                .ok()
        })
        .expect("compiled golish-memory-app rlib");
    let output = std::process::Command::new("rustc")
        .arg("--edition=2021")
        .arg(format!(
            "{}/tests/ui/trusted_context_is_private.rs",
            env!("CARGO_MANIFEST_DIR")
        ))
        .arg("--crate-name=trusted_context_is_private")
        .arg("--extern")
        .arg(format!("golish_memory_app={}", memory_app.display()))
        .arg("-L")
        .arg(format!("dependency={}", deps.display()))
        .arg("--out-dir")
        .arg(std::env::temp_dir())
        .output()
        .expect("run rustc UI compile-fail test");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "forged context unexpectedly compiled"
    );
    assert!(
        stderr.contains("private"),
        "compile failure must be caused by private authorization fields: {stderr}"
    );
}

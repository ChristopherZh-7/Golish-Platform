use super::*;
use tempfile::TempDir;

#[test]
fn test_search_rust_function() {
    let source = "fn foo(x: i32) -> i32 { x + 1 }";
    let results = search_source(
        source,
        "fn $NAME($$$ARGS) -> $RET { $$$BODY }",
        SupportLang::Rust,
    );
    assert_eq!(results.len(), 1);
    assert!(results[0].text.contains("fn foo"));
}

#[test]
fn test_search_multiple_functions() {
    let source = r#"
fn add(a: i32, b: i32) -> i32 { a + b }
fn sub(a: i32, b: i32) -> i32 { a - b }
fn mul(a: i32, b: i32) -> i32 { a * b }
"#;
    let results = search_source(
        source,
        "fn $NAME($$$ARGS) -> $RET { $$$BODY }",
        SupportLang::Rust,
    );
    assert_eq!(results.len(), 3);
}

#[test]
fn test_search_javascript_arrow_function() {
    let source = "const add = (a, b) => a + b;";
    let results = search_source(source, "($$$ARGS) => $BODY", SupportLang::JavaScript);
    assert_eq!(results.len(), 1);
}

#[test]
fn test_search_python_function() {
    let source = r#"
def greet(name):
    return f'Hello, {name}'

def farewell(name):
    return f'Goodbye, {name}'
"#;
    let results = search_source(source, "return $EXPR", SupportLang::Python);
    assert_eq!(results.len(), 2);
}

#[test]
fn test_replace_rust_function_call() {
    let source = "println!(\"hello\");";
    let result = replace_source(
        source,
        "println!($MSG)",
        "log::info!($MSG)",
        SupportLang::Rust,
    );
    assert_eq!(result, "log::info!(\"hello\");");
}

#[test]
fn test_replace_javascript_console_log() {
    let source = "console.log('hello');";
    let result = replace_source(
        source,
        "console.log($MSG)",
        "logger.info($MSG)",
        SupportLang::JavaScript,
    );
    assert_eq!(result, "logger.info('hello');");
}

#[test]
fn test_replace_multiple_occurrences() {
    let source = r#"
console.log('first');
console.log('second');
console.log('third');
"#;
    let result = replace_source(
        source,
        "console.log($MSG)",
        "logger.info($MSG)",
        SupportLang::JavaScript,
    );
    assert!(result.contains("logger.info('first')"));
    assert!(result.contains("logger.info('second')"));
    assert!(result.contains("logger.info('third')"));
    assert!(!result.contains("console.log"));
}

#[test]
fn test_directory_search() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("main.rs"), "fn main() {}").unwrap();
    fs::write(tmp.path().join("lib.rs"), "fn helper() {}").unwrap();

    let result = search(tmp.path(), "fn $NAME() {}", None, Some("rust")).unwrap();
    assert_eq!(result.matches.len(), 2);
    assert_eq!(result.files_searched, 2);
}

#[test]
fn test_directory_search_with_subdirs() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir(tmp.path().join("src")).unwrap();
    fs::write(tmp.path().join("src/main.rs"), "fn main() {}").unwrap();
    fs::write(tmp.path().join("src/lib.rs"), "fn helper() {}").unwrap();

    let result = search(tmp.path(), "fn $NAME() {}", None, Some("rust")).unwrap();
    assert_eq!(result.matches.len(), 2);
}

#[test]
fn test_directory_replace() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("test.js"), "console.log('hello');").unwrap();

    let result = replace(
        tmp.path(),
        "console.log($MSG)",
        "logger.info($MSG)",
        "test.js",
        Some("javascript"),
    )
    .unwrap();

    assert_eq!(result.files_modified.len(), 1);
    assert_eq!(result.replacements_count, 1);

    let new_content = fs::read_to_string(tmp.path().join("test.js")).unwrap();
    assert_eq!(new_content, "logger.info('hello');");
}

#[test]
fn test_search_result_serialization() {
    let result = SearchResult {
        matches: vec![SearchMatch {
            file: "test.rs".to_string(),
            line: 1,
            column: 1,
            text: "fn foo()".to_string(),
            end_line: 1,
            end_column: 9,
        }],
        files_searched: 1,
        error: None,
    };

    let json = serde_json::to_string(&result).unwrap();
    assert!(json.contains("test.rs"));
    assert!(json.contains("fn foo()"));
}

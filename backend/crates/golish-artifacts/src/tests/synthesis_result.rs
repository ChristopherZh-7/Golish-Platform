use super::*;

#[test]
fn creates_synthesis_result() {
    let result = ArtifactSynthesisResult {
        content: "# Updated README".to_string(),
        backend: "template".to_string(),
    };

    assert_eq!(result.content, "# Updated README");
    assert_eq!(result.backend, "template");
}

#[test]
fn synthesis_result_serializes() {
    let result = ArtifactSynthesisResult {
        content: "# Content".to_string(),
        backend: "openai".to_string(),
    };

    let json = serde_json::to_string(&result).unwrap();
    assert!(json.contains("\"content\":\"# Content\""));
    assert!(json.contains("\"backend\":\"openai\""));
}

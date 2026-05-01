use super::*;

#[test]
fn backend_from_str_template() {
    let backend: ArtifactSynthesisBackend = "template".parse().unwrap();
    assert_eq!(backend, ArtifactSynthesisBackend::Template);
}

#[test]
fn backend_from_str_vertex() {
    let backend: ArtifactSynthesisBackend = "vertex_anthropic".parse().unwrap();
    assert_eq!(backend, ArtifactSynthesisBackend::VertexAnthropic);

    // Short form
    let backend: ArtifactSynthesisBackend = "vertex".parse().unwrap();
    assert_eq!(backend, ArtifactSynthesisBackend::VertexAnthropic);
}

#[test]
fn backend_from_str_openai() {
    let backend: ArtifactSynthesisBackend = "openai".parse().unwrap();
    assert_eq!(backend, ArtifactSynthesisBackend::OpenAi);
}

#[test]
fn backend_from_str_grok() {
    let backend: ArtifactSynthesisBackend = "grok".parse().unwrap();
    assert_eq!(backend, ArtifactSynthesisBackend::Grok);
}

#[test]
fn backend_from_str_invalid() {
    let result: Result<ArtifactSynthesisBackend, _> = "invalid".parse();
    assert!(result.is_err());
}

#[test]
fn backend_display() {
    assert_eq!(ArtifactSynthesisBackend::Template.to_string(), "template");
    assert_eq!(
        ArtifactSynthesisBackend::VertexAnthropic.to_string(),
        "vertex_anthropic"
    );
    assert_eq!(ArtifactSynthesisBackend::OpenAi.to_string(), "openai");
    assert_eq!(ArtifactSynthesisBackend::Grok.to_string(), "grok");
}

#[test]
fn config_default_is_template() {
    let config = ArtifactSynthesisConfig::default();
    assert_eq!(config.backend, ArtifactSynthesisBackend::Template);
    assert!(!config.uses_llm());
}

#[test]
fn config_uses_llm_when_not_template() {
    let mut config = ArtifactSynthesisConfig {
        backend: ArtifactSynthesisBackend::OpenAi,
        ..Default::default()
    };
    assert!(config.uses_llm());

    config.backend = ArtifactSynthesisBackend::VertexAnthropic;
    assert!(config.uses_llm());

    config.backend = ArtifactSynthesisBackend::Grok;
    assert!(config.uses_llm());
}

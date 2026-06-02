//! Render a profile-projected Operation DAG as a Mermaid flowchart (the "图"
//! the UI / docs can display). Borrows metalcraft's `to_mermaid` rendering
//! style (graph_engine) but emits directly from [`AllowedDag`] so no `Reducer`
//! state type is needed — the operation graph is a pure stage topology.

use super::graph_engine::{END, START};
use super::operation_graph::{base_operation_graph, AllowedDag};
use super::profile::Profile;

/// Render an already-projected [`AllowedDag`] to a Mermaid `flowchart TD`.
///
/// `__start__ --> <entry>` for each entry stage, one line per edge, and
/// `<terminal> --> __end__` for each terminal stage.
pub fn dag_to_mermaid(dag: &AllowedDag) -> String {
    let mut lines = vec!["flowchart TD".to_string()];
    for entry in dag.entry_points() {
        lines.push(format!("    {START} --> {}", entry.as_str()));
    }
    for e in &dag.edges {
        lines.push(format!("    {} --> {}", e.from.as_str(), e.to.as_str()));
    }
    for terminal in dag.terminals() {
        lines.push(format!("    {} --> {END}", terminal.as_str()));
    }
    lines.join("\n")
}

/// Convenience: load the embedded base operation graph, project it onto
/// `profile`'s allowed stage set, and render to Mermaid. On graph-load failure
/// returns a Mermaid comment line (never panics) so callers can surface it.
pub fn operation_mermaid_for_profile(profile: &Profile) -> String {
    match base_operation_graph() {
        Ok(graph) => dag_to_mermaid(&graph.project(&profile.allowed_stage_set())),
        Err(e) => format!("%% operation graph load failed: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::profile::load_profile_from_json;

    const ASSESSMENT_JSON: &str =
        include_str!("../../../../../resources/harness/profiles/assessment.json");

    #[test]
    fn assessment_mermaid_has_start_edges_and_terminal() {
        let profile = load_profile_from_json(ASSESSMENT_JSON).expect("assessment profile");
        let m = operation_mermaid_for_profile(&profile);
        assert!(m.starts_with("flowchart TD"));
        assert!(m.contains("__start__ --> scoping"));
        assert!(m.contains("scoping --> target_intel"));
        assert!(m.contains("external_attack_surface --> enumeration"));
        assert!(m.contains("reporting --> __end__"));
    }
}

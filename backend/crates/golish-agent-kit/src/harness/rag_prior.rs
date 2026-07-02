//! P3 · knowledge + continuous: RAG prior retrieval + knowledge-graph prior +
//! continuous feedback.
//!
//! - **P3-a** (`retrieve_wiki_prior`): pull prior writeups/PoCs from the wiki KB
//!   (`DbRepoProvider::wiki_search_fts`) so the agent sees known exploits before
//!   testing — "测漏洞前自动检索 writeup".
//! - **P3-b** (`retrieve_graph_prior`): pull related facts from the knowledge
//!   graph (`GraphKnowledgeBase::search_entities`, deepening borrowed from
//!   PentAGI's Graphiti). `retrieve_prior_knowledge` unifies wiki + graph.
//! - **P3-c** (`feed_findings_to_graph`): after a stage, write findings back into
//!   the graph so the next operation's prior retrieval can use them (continuous).
//!
//! Retrieval/render are the testable core; injecting `render_prior_knowledge`
//! into the vuln_triage/verification stage prompt is the live-wiring follow-up
//! (same "SDK first, wire after" approach as P2 eval/guardrail).

use serde_json::{json, Value};
use uuid::Uuid;

use crate::db_traits::DbRepoProvider;
use crate::harness::types::StageDeliverable;
use crate::tool_executors::graph_trait::GraphKnowledgeBase;

const SNIPPET_MAX: usize = 280;

/// One retrieved prior-knowledge item (writeup / PoC / graph fact).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriorWriteup {
    pub source: String,
    pub title: String,
    pub snippet: String,
}

/// Bundle of prior knowledge retrieved for a query.
#[derive(Debug, Clone, Default)]
pub struct PriorKnowledge {
    pub writeups: Vec<PriorWriteup>,
}

impl PriorKnowledge {
    pub fn is_empty(&self) -> bool {
        self.writeups.is_empty()
    }
}

fn truncate(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() <= SNIPPET_MAX {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(SNIPPET_MAX).collect();
        out.push('…');
        out
    }
}

/// Parse a `wiki_search_fts` JSON value into writeups. Defensive about shape:
/// accepts a top-level array or `{results|pages|items: [...]}`, and reads
/// title from `title|path|name|cve_id` and snippet from
/// `snippet|summary|content|body|description`.
fn parse_wiki_value(v: &Value) -> Vec<PriorWriteup> {
    let arr = v
        .as_array()
        .or_else(|| v.get("results").and_then(Value::as_array))
        .or_else(|| v.get("pages").and_then(Value::as_array))
        .or_else(|| v.get("items").and_then(Value::as_array));
    let Some(arr) = arr else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|item| {
            let obj = item.as_object()?;
            let pick = |keys: &[&str]| -> Option<String> {
                keys.iter()
                    .find_map(|k| obj.get(*k).and_then(Value::as_str))
                    .map(str::to_string)
            };
            let title = pick(&["title", "path", "name", "cve_id"])?;
            let snippet =
                pick(&["snippet", "summary", "content", "body", "description"]).unwrap_or_default();
            Some(PriorWriteup {
                source: "wiki".to_string(),
                title,
                snippet: truncate(&snippet),
            })
        })
        .collect()
}

/// P3-a · retrieve prior writeups from the wiki KB. Errors degrade to empty
/// (prior knowledge is best-effort priming, never blocks the stage).
pub async fn retrieve_wiki_prior(
    repo: &dyn DbRepoProvider,
    query: &str,
    limit: i64,
) -> PriorKnowledge {
    let writeups = match repo.wiki_search_fts(query, limit).await {
        Ok(v) => parse_wiki_value(&v),
        Err(_) => Vec::new(),
    };
    PriorKnowledge { writeups }
}

/// P3-b · retrieve related facts from the knowledge graph.
pub async fn retrieve_graph_prior(
    graph: &dyn GraphKnowledgeBase,
    query: &str,
    limit: i64,
) -> PriorKnowledge {
    let writeups = match graph.search_entities(query, None, limit).await {
        Ok(entities) => entities
            .into_iter()
            .map(|e| PriorWriteup {
                source: format!("graph:{}", e.entity_type),
                title: e.name,
                snippet: truncate(&e.properties.to_string()),
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    PriorKnowledge { writeups }
}

/// Unified prior retrieval: wiki KB + (optional) knowledge graph.
pub async fn retrieve_prior_knowledge(
    repo: &dyn DbRepoProvider,
    graph: Option<&dyn GraphKnowledgeBase>,
    query: &str,
    limit: i64,
) -> PriorKnowledge {
    let mut pk = retrieve_wiki_prior(repo, query, limit).await;
    if let Some(g) = graph {
        pk.writeups
            .extend(retrieve_graph_prior(g, query, limit).await.writeups);
    }
    pk
}

/// Render prior knowledge as a Markdown block to prepend to a stage prompt.
/// Empty input → empty string (no section emitted).
pub fn render_prior_knowledge(pk: &PriorKnowledge) -> String {
    if pk.is_empty() {
        return String::new();
    }
    let mut s = String::from(
        "## PRIOR KNOWLEDGE (retrieved writeups / facts — consult before testing)\n\n",
    );
    for w in &pk.writeups {
        s.push_str(&format!("- [{}] **{}**", w.source, w.title));
        if !w.snippet.is_empty() {
            s.push_str(&format!(": {}", w.snippet));
        }
        s.push('\n');
    }
    s
}

/// P3-c · continuous: write a stage's findings into the knowledge graph so the
/// next operation's prior retrieval can surface them. Returns how many were
/// upserted. Best-effort (failures are skipped, never block the stage).
pub async fn feed_findings_to_graph(
    graph: &dyn GraphKnowledgeBase,
    deliverable: &StageDeliverable,
    session_id: Option<Uuid>,
) -> usize {
    let mut n = 0usize;
    for f in &deliverable.findings {
        let props = json!({
            "kind": f.kind,
            "severity": f.severity,
            "evidence_ref_count": f.evidence_refs.len(),
            "stage_id": deliverable.stage_id,
        });
        if graph
            .upsert_entity("finding", &f.subject, props, session_id)
            .await
            .is_ok()
        {
            n += 1;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::types::{FindingSeverity, HarnessFinding};
    use crate::tool_executors::graph_trait::{GraphEntityView, GraphRelationView};
    use async_trait::async_trait;
    use std::sync::Mutex;

    #[test]
    fn parse_wiki_array_and_results_shapes() {
        let arr = json!([
            {"title": "CVE-2021-44228 Log4Shell", "snippet": "JNDI lookup RCE"},
            {"path": "wiki/struts", "content": "OGNL injection"}
        ]);
        let w = parse_wiki_value(&arr);
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].title, "CVE-2021-44228 Log4Shell");
        assert_eq!(w[0].source, "wiki");

        let wrapped = json!({"results": [{"name": "x", "summary": "y"}]});
        assert_eq!(parse_wiki_value(&wrapped).len(), 1);

        // unknown shape → empty, no panic
        assert!(parse_wiki_value(&json!({"foo": 1})).is_empty());
    }

    #[test]
    fn render_emits_section_or_empty() {
        assert_eq!(render_prior_knowledge(&PriorKnowledge::default()), "");
        let pk = PriorKnowledge {
            writeups: vec![PriorWriteup {
                source: "wiki".into(),
                title: "Log4Shell".into(),
                snippet: "JNDI RCE".into(),
            }],
        };
        let r = render_prior_knowledge(&pk);
        assert!(r.contains("PRIOR KNOWLEDGE"));
        assert!(r.contains("[wiki]"));
        assert!(r.contains("Log4Shell"));
    }

    struct MockGraph {
        upserts: Mutex<Vec<String>>,
    }
    #[async_trait]
    impl GraphKnowledgeBase for MockGraph {
        async fn upsert_entity(
            &self,
            entity_type: &str,
            name: &str,
            _properties: Value,
            session_id: Option<Uuid>,
        ) -> anyhow::Result<GraphEntityView> {
            self.upserts
                .lock()
                .unwrap()
                .push(format!("{entity_type}:{name}"));
            Ok(GraphEntityView {
                id: Uuid::new_v4(),
                entity_type: entity_type.to_string(),
                name: name.to_string(),
                properties: json!({}),
                session_id,
                project_id: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
        }
        async fn upsert_relation(
            &self,
            _from_id: Uuid,
            _to_id: Uuid,
            _relation_type: &str,
            _properties: Value,
        ) -> anyhow::Result<GraphRelationView> {
            unimplemented!()
        }
        async fn search_entities(
            &self,
            query: &str,
            _entity_type: Option<&str>,
            _limit: i64,
        ) -> anyhow::Result<Vec<GraphEntityView>> {
            Ok(vec![GraphEntityView {
                id: Uuid::new_v4(),
                entity_type: "cve".to_string(),
                name: format!("match:{query}"),
                properties: json!({"note": "graph fact"}),
                session_id: None,
                project_id: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            }])
        }
        async fn get_neighbors(
            &self,
            _entity_id: Uuid,
            _relation_type: Option<&str>,
        ) -> anyhow::Result<Vec<(GraphRelationView, GraphEntityView)>> {
            unimplemented!()
        }
        async fn find_attack_paths(
            &self,
            _from_id: Uuid,
            _max_depth: i32,
        ) -> anyhow::Result<Vec<Vec<GraphEntityView>>> {
            unimplemented!()
        }
    }

    #[tokio::test]
    async fn graph_prior_maps_entities_to_writeups() {
        let g = MockGraph {
            upserts: Mutex::new(vec![]),
        };
        let pk = retrieve_graph_prior(&g, "log4j", 5).await;
        assert_eq!(pk.writeups.len(), 1);
        assert_eq!(pk.writeups[0].source, "graph:cve");
        assert!(pk.writeups[0].title.contains("log4j"));
    }

    #[tokio::test]
    async fn feed_findings_upserts_each_finding() {
        let g = MockGraph {
            upserts: Mutex::new(vec![]),
        };
        let d = StageDeliverable {
            stage_id: "verification".to_string(),
            stage_run_id: Uuid::new_v4(),
            claims: vec![],
            evidence_refs: vec![],
            skipped_checks: vec![],
            findings: vec![
                HarnessFinding {
                    finding_id: Uuid::new_v4(),
                    kind: "rce".to_string(),
                    subject: "api.example.com".to_string(),
                    severity: FindingSeverity::Critical,
                    evidence_refs: vec![],
                    technique: None,
                },
                HarnessFinding {
                    finding_id: Uuid::new_v4(),
                    kind: "xss".to_string(),
                    subject: "www.example.com".to_string(),
                    severity: FindingSeverity::Medium,
                    evidence_refs: vec![],
                    technique: None,
                },
            ],
            required_checks_done: vec![],
            coverage: vec![],
            candidates: vec![],
        };
        let n = feed_findings_to_graph(&g, &d, None).await;
        assert_eq!(n, 2);
        assert_eq!(g.upserts.lock().unwrap().len(), 2);
    }
}

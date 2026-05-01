//! `ToolCategory` enum: semantic grouping for tools.

use serde::{Deserialize, Serialize};

use super::ToolName;

///
/// This differs from the routing-based `ToolCategory` in `tool_execution.rs` -
/// this is about semantic grouping (what the tool *does*), not how it's routed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCategory {
    /// File read/write/edit operations
    FileOps,
    /// Directory listing and search operations
    DirectoryOps,
    /// Shell command execution
    Shell,
    /// Web fetching and search operations
    Web,
    /// Task planning operations
    Planning,
    /// Code indexing and analysis
    Indexer,
    /// AST-based code operations
    Ast,
    /// Multi-step workflow execution
    Workflow,
    /// Sub-agent delegation
    SubAgent,
    /// Vulnerability knowledge base operations
    KnowledgeBase,
    /// Security analysis and pentest operations
    SecurityAnalysis,
    /// Graph knowledge base operations (typed entities/relations)
    Graph,
}

impl ToolCategory {
    /// Get all tool names in this category.
    pub fn tools(&self) -> &'static [ToolName] {
        match self {
            Self::FileOps => &[
                ToolName::ReadFile,
                ToolName::WriteFile,
                ToolName::EditFile,
                ToolName::CreateFile,
                ToolName::DeleteFile,
            ],
            Self::DirectoryOps => &[
                ToolName::ListFiles,
                ToolName::ListDirectory,
                ToolName::GrepFile,
            ],
            Self::Shell => &[ToolName::RunPtyCmd, ToolName::RunCommand],
            Self::Web => &[
                ToolName::WebFetch,
                ToolName::WebSearch,
                ToolName::WebSearchAnswer,
                ToolName::WebExtract,
                ToolName::WebCrawl,
                ToolName::WebMap,
            ],
            Self::Planning => &[ToolName::UpdatePlan],
            Self::Indexer => &[
                ToolName::IndexerSearchCode,
                ToolName::IndexerSearchFiles,
                ToolName::IndexerAnalyzeFile,
                ToolName::IndexerExtractSymbols,
                ToolName::IndexerGetMetrics,
                ToolName::IndexerDetectLanguage,
            ],
            Self::Ast => &[ToolName::AstGrep, ToolName::AstGrepReplace],
            Self::Workflow => &[ToolName::RunWorkflow],
            Self::SubAgent => &[], // Dynamic, not enumerable
            Self::KnowledgeBase => &[
                ToolName::SearchKnowledgeBase,
                ToolName::WriteKnowledge,
                ToolName::ReadKnowledge,
                ToolName::IngestCve,
                ToolName::SavePoc,
                ToolName::ListCvesWithPocs,
                ToolName::ListUnresearchedCves,
                ToolName::PocStats,
            ],
            Self::SecurityAnalysis => &[
                ToolName::LogOperation,
                ToolName::DiscoverApis,
                ToolName::SaveJsAnalysis,
                ToolName::FingerprintTarget,
                ToolName::LogScanResult,
                ToolName::QueryTargetData,
                ToolName::SearchExploits,
            ],
            Self::Graph => &[
                ToolName::GraphAddEntity,
                ToolName::GraphAddRelation,
                ToolName::GraphSearch,
                ToolName::GraphNeighbors,
                ToolName::GraphAttackPaths,
            ],
        }
    }

    /// Check if this category contains read-only tools.
    pub fn is_read_only(&self) -> bool {
        matches!(self, Self::DirectoryOps | Self::Indexer)
    }
}

impl std::fmt::Display for ToolCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FileOps => write!(f, "file_ops"),
            Self::DirectoryOps => write!(f, "directory_ops"),
            Self::Shell => write!(f, "shell"),
            Self::Web => write!(f, "web"),
            Self::Planning => write!(f, "planning"),
            Self::Indexer => write!(f, "indexer"),
            Self::Ast => write!(f, "ast"),
            Self::Workflow => write!(f, "workflow"),
            Self::SubAgent => write!(f, "sub_agent"),
            Self::KnowledgeBase => write!(f, "knowledge_base"),
            Self::SecurityAnalysis => write!(f, "security_analysis"),
            Self::Graph => write!(f, "graph"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_name_roundtrip() {
        let tools = [
            ToolName::ReadFile,
            ToolName::WriteFile,
            ToolName::EditFile,
            ToolName::RunPtyCmd,
            ToolName::WebFetch,
            ToolName::UpdatePlan,
            ToolName::IndexerSearchCode,
            ToolName::AstGrep,
        ];

        for tool in tools {
            let s = tool.as_str();
            let parsed = ToolName::from_str(s);
            assert_eq!(parsed, Some(tool), "Roundtrip failed for {:?}", tool);
        }
    }

    #[test]
    fn test_tool_name_from_str_unknown() {
        assert_eq!(ToolName::from_str("unknown_tool"), None);
        assert_eq!(ToolName::from_str("sub_agent_coder"), None);
        assert_eq!(ToolName::from_str(""), None);
    }

    #[test]
    fn test_tool_name_aliases() {
        // tavily_* should map to web_*
        assert_eq!(
            ToolName::from_str("tavily_search"),
            Some(ToolName::WebSearch)
        );
        assert_eq!(ToolName::from_str("web_search"), Some(ToolName::WebSearch));
        assert_eq!(
            ToolName::from_str("tavily_extract"),
            Some(ToolName::WebExtract)
        );
    }

    #[test]
    fn test_tool_category() {
        assert_eq!(ToolName::ReadFile.category(), ToolCategory::FileOps);
        assert_eq!(ToolName::WriteFile.category(), ToolCategory::FileOps);
        assert_eq!(ToolName::RunPtyCmd.category(), ToolCategory::Shell);
        assert_eq!(ToolName::WebFetch.category(), ToolCategory::Web);
        assert_eq!(ToolName::UpdatePlan.category(), ToolCategory::Planning);
        assert_eq!(
            ToolName::IndexerSearchCode.category(),
            ToolCategory::Indexer
        );
    }

    #[test]
    fn test_is_read_only() {
        assert!(ToolName::ReadFile.is_read_only());
        assert!(ToolName::ListFiles.is_read_only());
        assert!(ToolName::GrepFile.is_read_only());
        assert!(ToolName::WebSearch.is_read_only());
        assert!(ToolName::IndexerSearchCode.is_read_only());
        assert!(ToolName::AstGrep.is_read_only());

        assert!(!ToolName::WriteFile.is_read_only());
        assert!(!ToolName::EditFile.is_read_only());
        assert!(!ToolName::RunPtyCmd.is_read_only());
        assert!(!ToolName::AstGrepReplace.is_read_only());
    }

    #[test]
    fn test_sub_agent_detection() {
        assert!(ToolName::is_sub_agent_tool("sub_agent_coder"));
        assert!(ToolName::is_sub_agent_tool("sub_agent_researcher"));
        assert!(!ToolName::is_sub_agent_tool("read_file"));
        assert!(!ToolName::is_sub_agent_tool("sub_agent"));

        assert_eq!(ToolName::sub_agent_id("sub_agent_coder"), Some("coder"));
        assert_eq!(ToolName::sub_agent_id("read_file"), None);
    }

    #[test]
    fn test_category_tools() {
        let file_ops = ToolCategory::FileOps.tools();
        assert!(file_ops.contains(&ToolName::ReadFile));
        assert!(file_ops.contains(&ToolName::WriteFile));
        assert!(!file_ops.contains(&ToolName::RunPtyCmd));

        let shell = ToolCategory::Shell.tools();
        assert!(shell.contains(&ToolName::RunPtyCmd));
        assert!(shell.contains(&ToolName::RunCommand));
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", ToolName::ReadFile), "read_file");
        assert_eq!(format!("{}", ToolCategory::FileOps), "file_ops");
    }

    #[test]
    fn test_serde_roundtrip() {
        let tool = ToolName::ReadFile;
        let json = serde_json::to_string(&tool).unwrap();
        assert_eq!(json, "\"read_file\"");

        let parsed: ToolName = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, tool);
    }
}

//! `ToolName` enum: type-safe tool identifiers.

use serde::{Deserialize, Serialize};

use super::ToolCategory;

/// Enumeration of all known tool names.
///
/// This provides type-safe tool identification, preventing typos and enabling
/// exhaustive matching in tool handlers and hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ToolName {
    // === File Operations ===
    /// Read contents of a file
    ReadFile,
    /// Write contents to a file (overwrite)
    WriteFile,
    /// Edit a file with search/replace
    EditFile,
    /// Create a new file
    CreateFile,
    /// Delete a file
    DeleteFile,

    // === Directory Operations ===
    /// List files matching a pattern
    ListFiles,
    /// List directory contents
    ListDirectory,
    /// Search file contents with grep
    GrepFile,

    // === Shell Execution ===
    /// Execute a command in PTY
    RunPtyCmd,
    /// Alias for RunPtyCmd (user-friendly name)
    RunCommand,

    // === Web Operations ===
    /// Fetch and extract web content
    WebFetch,
    /// Web search via Tavily
    WebSearch,
    /// Web search with answer via Tavily
    WebSearchAnswer,
    /// Extract content from URLs via Tavily
    WebExtract,
    /// Crawl website via Tavily
    WebCrawl,
    /// Map website structure via Tavily
    WebMap,

    // === Planning ===
    /// Update task plan
    UpdatePlan,

    // === Code Indexer ===
    /// Search code in index
    IndexerSearchCode,
    /// Search files in index
    IndexerSearchFiles,
    /// Analyze a file's structure
    IndexerAnalyzeFile,
    /// Extract symbols from a file
    IndexerExtractSymbols,
    /// Get code metrics for a file
    IndexerGetMetrics,
    /// Detect file language
    IndexerDetectLanguage,

    // === AST Operations ===
    /// AST-based code search
    AstGrep,
    /// AST-based code replacement
    AstGrepReplace,

    // === Workflow ===
    /// Execute a workflow
    RunWorkflow,

    // === Knowledge Base ===
    /// Search the vulnerability knowledge base
    SearchKnowledgeBase,
    /// Write/update a knowledge base page
    WriteKnowledge,
    /// Read a knowledge base page
    ReadKnowledge,
    /// Ingest a CVE into the knowledge base
    IngestCve,
    SavePoc,
    ListCvesWithPocs,
    ListUnresearchedCves,
    PocStats,

    // === Security Analysis ===
    /// Log a pentest operation
    LogOperation,
    /// Discover API endpoints from target
    DiscoverApis,
    /// Save JavaScript file analysis results
    SaveJsAnalysis,
    /// Fingerprint target technology stack
    FingerprintTarget,
    /// Log a passive scan / manual test result
    LogScanResult,
    /// Query aggregated target data (assets, endpoints, fingerprints)
    QueryTargetData,

    // === Graph Knowledge Base ===
    /// Add or update an entity in the knowledge graph
    GraphAddEntity,
    /// Add or update a relation between entities
    GraphAddRelation,
    /// Search entities in the knowledge graph
    GraphSearch,
    /// Get neighboring entities and relations
    GraphNeighbors,
    /// Find attack paths between entities
    GraphAttackPaths,

    // === Vulnerability Database ===
    /// Search exploits and CVEs via Sploitus
    SearchExploits,
}

impl ToolName {
    /// Get the string representation of the tool name.
    ///
    /// This returns the exact string that matches what the LLM requests.
    pub fn as_str(&self) -> &'static str {
        match self {
            // File Operations
            Self::ReadFile => "read_file",
            Self::WriteFile => "write_file",
            Self::EditFile => "edit_file",
            Self::CreateFile => "create_file",
            Self::DeleteFile => "delete_file",

            // Directory Operations
            Self::ListFiles => "list_files",
            Self::ListDirectory => "list_directory",
            Self::GrepFile => "grep_file",

            // Shell
            Self::RunPtyCmd => "run_pty_cmd",
            Self::RunCommand => "run_command",

            // Web
            Self::WebFetch => "web_fetch",
            Self::WebSearch => "web_search",
            Self::WebSearchAnswer => "web_search_answer",
            Self::WebExtract => "web_extract",
            Self::WebCrawl => "web_crawl",
            Self::WebMap => "web_map",

            // Planning
            Self::UpdatePlan => "update_plan",

            // Indexer
            Self::IndexerSearchCode => "indexer_search_code",
            Self::IndexerSearchFiles => "indexer_search_files",
            Self::IndexerAnalyzeFile => "indexer_analyze_file",
            Self::IndexerExtractSymbols => "indexer_extract_symbols",
            Self::IndexerGetMetrics => "indexer_get_metrics",
            Self::IndexerDetectLanguage => "indexer_detect_language",

            // AST
            Self::AstGrep => "ast_grep",
            Self::AstGrepReplace => "ast_grep_replace",

            // Workflow
            Self::RunWorkflow => "run_workflow",

            // Knowledge Base
            Self::SearchKnowledgeBase => "search_knowledge_base",
            Self::WriteKnowledge => "write_knowledge",
            Self::ReadKnowledge => "read_knowledge",
            Self::IngestCve => "ingest_cve",
            Self::SavePoc => "save_poc",
            Self::ListCvesWithPocs => "list_cves_with_pocs",
            Self::ListUnresearchedCves => "list_unresearched_cves",
            Self::PocStats => "poc_stats",

            // Security Analysis
            Self::LogOperation => "log_operation",
            Self::DiscoverApis => "discover_apis",
            Self::SaveJsAnalysis => "save_js_analysis",
            Self::FingerprintTarget => "fingerprint_target",
            Self::LogScanResult => "log_scan_result",
            Self::QueryTargetData => "query_target_data",

            // Graph Knowledge Base
            Self::GraphAddEntity => "graph_add_entity",
            Self::GraphAddRelation => "graph_add_relation",
            Self::GraphSearch => "graph_search",
            Self::GraphNeighbors => "graph_neighbors",
            Self::GraphAttackPaths => "graph_attack_paths",

            // Vulnerability Database
            Self::SearchExploits => "search_exploits",
        }
    }

    /// Parse a tool name from a string.
    ///
    /// Returns `None` for unknown tool names (e.g., dynamic sub-agent tools).
    /// Note: We intentionally don't implement `FromStr` because this returns `Option`
    /// rather than `Result`, as unknown tool names are expected (not errors).
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            // File Operations
            "read_file" => Some(Self::ReadFile),
            "write_file" => Some(Self::WriteFile),
            "edit_file" => Some(Self::EditFile),
            "create_file" => Some(Self::CreateFile),
            "delete_file" => Some(Self::DeleteFile),

            // Directory Operations
            "list_files" => Some(Self::ListFiles),
            "list_directory" => Some(Self::ListDirectory),
            "grep_file" => Some(Self::GrepFile),

            // Shell
            "run_pty_cmd" => Some(Self::RunPtyCmd),
            "run_command" => Some(Self::RunCommand),

            // Web
            "web_fetch" => Some(Self::WebFetch),
            "web_search" | "tavily_search" => Some(Self::WebSearch),
            "web_search_answer" | "tavily_search_answer" => Some(Self::WebSearchAnswer),
            "web_extract" | "tavily_extract" => Some(Self::WebExtract),
            "web_crawl" | "tavily_crawl" => Some(Self::WebCrawl),
            "web_map" | "tavily_map" => Some(Self::WebMap),

            // Planning
            "update_plan" => Some(Self::UpdatePlan),

            // Indexer
            "indexer_search_code" => Some(Self::IndexerSearchCode),
            "indexer_search_files" => Some(Self::IndexerSearchFiles),
            "indexer_analyze_file" => Some(Self::IndexerAnalyzeFile),
            "indexer_extract_symbols" => Some(Self::IndexerExtractSymbols),
            "indexer_get_metrics" => Some(Self::IndexerGetMetrics),
            "indexer_detect_language" => Some(Self::IndexerDetectLanguage),

            // AST
            "ast_grep" => Some(Self::AstGrep),
            "ast_grep_replace" => Some(Self::AstGrepReplace),

            // Workflow
            "run_workflow" => Some(Self::RunWorkflow),

            // Knowledge Base
            "search_knowledge_base" => Some(Self::SearchKnowledgeBase),
            "write_knowledge" => Some(Self::WriteKnowledge),
            "read_knowledge" => Some(Self::ReadKnowledge),
            "ingest_cve" => Some(Self::IngestCve),
            "save_poc" => Some(Self::SavePoc),
            "list_cves_with_pocs" => Some(Self::ListCvesWithPocs),
            "list_unresearched_cves" => Some(Self::ListUnresearchedCves),
            "poc_stats" => Some(Self::PocStats),

            // Security Analysis
            "log_operation" => Some(Self::LogOperation),
            "discover_apis" => Some(Self::DiscoverApis),
            "save_js_analysis" => Some(Self::SaveJsAnalysis),
            "fingerprint_target" => Some(Self::FingerprintTarget),
            "log_scan_result" => Some(Self::LogScanResult),
            "query_target_data" => Some(Self::QueryTargetData),

            // Graph Knowledge Base
            "graph_add_entity" => Some(Self::GraphAddEntity),
            "graph_add_relation" => Some(Self::GraphAddRelation),
            "graph_search" => Some(Self::GraphSearch),
            "graph_neighbors" => Some(Self::GraphNeighbors),
            "graph_attack_paths" => Some(Self::GraphAttackPaths),

            // Vulnerability Database
            "search_exploits" => Some(Self::SearchExploits),

            // Unknown (includes dynamic sub-agent tools like "sub_agent_*")
            _ => None,
        }
    }

    /// Get the semantic category of this tool.
    pub fn category(&self) -> ToolCategory {
        match self {
            // File Operations
            Self::ReadFile
            | Self::WriteFile
            | Self::EditFile
            | Self::CreateFile
            | Self::DeleteFile => ToolCategory::FileOps,

            // Directory Operations
            Self::ListFiles | Self::ListDirectory | Self::GrepFile => ToolCategory::DirectoryOps,

            // Shell
            Self::RunPtyCmd | Self::RunCommand => ToolCategory::Shell,

            // Web
            Self::WebFetch
            | Self::WebSearch
            | Self::WebSearchAnswer
            | Self::WebExtract
            | Self::WebCrawl
            | Self::WebMap => ToolCategory::Web,

            // Planning
            Self::UpdatePlan => ToolCategory::Planning,

            // Indexer
            Self::IndexerSearchCode
            | Self::IndexerSearchFiles
            | Self::IndexerAnalyzeFile
            | Self::IndexerExtractSymbols
            | Self::IndexerGetMetrics
            | Self::IndexerDetectLanguage => ToolCategory::Indexer,

            // AST
            Self::AstGrep | Self::AstGrepReplace => ToolCategory::Ast,

            // Workflow
            Self::RunWorkflow => ToolCategory::Workflow,

            // Knowledge Base
            Self::SearchKnowledgeBase
            | Self::WriteKnowledge
            | Self::ReadKnowledge
            | Self::IngestCve
            | Self::SavePoc
            | Self::ListCvesWithPocs
            | Self::ListUnresearchedCves
            | Self::PocStats => ToolCategory::KnowledgeBase,

            // Security Analysis
            Self::LogOperation
            | Self::DiscoverApis
            | Self::SaveJsAnalysis
            | Self::FingerprintTarget
            | Self::LogScanResult
            | Self::QueryTargetData
            | Self::SearchExploits => ToolCategory::SecurityAnalysis,

            // Graph Knowledge Base
            Self::GraphAddEntity
            | Self::GraphAddRelation
            | Self::GraphSearch
            | Self::GraphNeighbors
            | Self::GraphAttackPaths => ToolCategory::Graph,
        }
    }

    /// Check if this tool is read-only (doesn't modify files or execute commands).
    pub fn is_read_only(&self) -> bool {
        matches!(
            self,
            Self::ReadFile
                | Self::ListFiles
                | Self::ListDirectory
                | Self::GrepFile
                | Self::WebFetch
                | Self::WebSearch
                | Self::WebSearchAnswer
                | Self::WebExtract
                | Self::WebCrawl
                | Self::WebMap
                | Self::IndexerSearchCode
                | Self::IndexerSearchFiles
                | Self::IndexerAnalyzeFile
                | Self::IndexerExtractSymbols
                | Self::IndexerGetMetrics
                | Self::IndexerDetectLanguage
                | Self::AstGrep
                | Self::SearchKnowledgeBase
                | Self::ReadKnowledge
                | Self::QueryTargetData
                | Self::GraphSearch
                | Self::GraphNeighbors
                | Self::GraphAttackPaths
                | Self::SearchExploits
        )
    }

    /// Check if this is a sub-agent tool name.
    ///
    /// Sub-agent tools are dynamically named as "sub_agent_<id>".
    pub fn is_sub_agent_tool(name: &str) -> bool {
        name.starts_with("sub_agent_")
    }

    /// Extract the sub-agent ID from a sub-agent tool name.
    ///
    /// Returns `None` if the name is not a sub-agent tool.
    pub fn sub_agent_id(name: &str) -> Option<&str> {
        name.strip_prefix("sub_agent_")
    }
}

impl std::fmt::Display for ToolName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl AsRef<str> for ToolName {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

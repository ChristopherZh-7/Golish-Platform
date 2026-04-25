//! Read-side helpers: list/find/load persisted session archives + extract
//! sidecar metadata for display in the session picker.

use std::path::{Path, PathBuf};

use anyhow::Result;

use golish_core::session::{
    find_session_by_identifier, list_recent_sessions as list_sessions_internal, MessageRole,
};

use crate::manager::GolishSessionManager;
use crate::types::{
    strip_xml_tags, GolishMessageRole, GolishSessionMessage, GolishSessionSnapshot,
    SessionListingInfo,
};


/// List recent sessions.
///
/// # Arguments
/// * `limit` - Maximum number of sessions to return (0 for all)
#[cfg_attr(not(feature = "tauri"), allow(dead_code))]
pub async fn list_recent_sessions(limit: usize) -> Result<Vec<SessionListingInfo>> {
    let listings = list_sessions_internal(limit).await?;

    Ok(listings
        .into_iter()
        .map(|listing| {
            let sidecar_meta = get_sidecar_session_meta(&listing.path);
            SessionListingInfo {
                identifier: listing.identifier(),
                path: listing.path.clone(),
                workspace_label: listing.snapshot.metadata.workspace_label.clone(),
                workspace_path: listing.snapshot.metadata.workspace_path.clone(),
                model: listing.snapshot.metadata.model.clone(),
                provider: listing.snapshot.metadata.provider.clone(),
                started_at: listing.snapshot.started_at,
                ended_at: listing.snapshot.ended_at,
                total_messages: listing.snapshot.total_messages,
                distinct_tools: listing.snapshot.distinct_tools.clone(),
                first_prompt_preview: listing.first_prompt_preview().map(|s| strip_xml_tags(&s)),
                first_reply_preview: listing.first_reply_preview().map(|s| strip_xml_tags(&s)),
                status: sidecar_meta.status,
                title: sidecar_meta.title,
            }
        })
        .collect())
}

/// Find a session by its identifier.
#[cfg_attr(not(feature = "tauri"), allow(dead_code))]
pub async fn find_session(identifier: &str) -> Result<Option<SessionListingInfo>> {
    let listing = find_session_by_identifier(identifier).await?;

    Ok(listing.map(|l| SessionListingInfo {
        identifier: l.identifier(),
        path: l.path.clone(),
        workspace_label: l.snapshot.metadata.workspace_label.clone(),
        workspace_path: l.snapshot.metadata.workspace_path.clone(),
        model: l.snapshot.metadata.model.clone(),
        provider: l.snapshot.metadata.provider.clone(),
        started_at: l.snapshot.started_at,
        ended_at: l.snapshot.ended_at,
        total_messages: l.snapshot.total_messages,
        distinct_tools: l.snapshot.distinct_tools.clone(),
        first_prompt_preview: l.first_prompt_preview().map(|s| strip_xml_tags(&s)),
        first_reply_preview: l.first_reply_preview().map(|s| strip_xml_tags(&s)),
        status: get_sidecar_session_meta(&l.path).status,
        title: get_sidecar_session_meta(&l.path).title,
    }))
}

/// Load a full session by identifier.
#[cfg_attr(not(feature = "tauri"), allow(dead_code))]
pub async fn load_session(identifier: &str) -> Result<Option<GolishSessionSnapshot>> {
    let listing = find_session_by_identifier(identifier).await?;

    Ok(listing.map(|l| {
        let messages = l
            .snapshot
            .messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    MessageRole::User => GolishMessageRole::User,
                    MessageRole::Assistant => GolishMessageRole::Assistant,
                    MessageRole::System => GolishMessageRole::System,
                    MessageRole::Tool => GolishMessageRole::Tool,
                };
                GolishSessionMessage {
                    role,
                    content: m.content.as_text().to_string(),
                    tool_call_id: m.tool_call_id.clone(),
                    tool_name: None,
                    tokens_used: None,
                }
            })
            .collect();

        // Read sidecar session ID from companion file
        let sidecar_session_id = GolishSessionManager::read_sidecar_session_id(&l.path);

        // Read agent mode from companion file
        let agent_mode = GolishSessionManager::read_agent_mode(&l.path);

        GolishSessionSnapshot {
            workspace_label: l.snapshot.metadata.workspace_label,
            workspace_path: l.snapshot.metadata.workspace_path,
            model: l.snapshot.metadata.model,
            provider: l.snapshot.metadata.provider,
            started_at: l.snapshot.started_at,
            ended_at: l.snapshot.ended_at,
            total_messages: l.snapshot.total_messages,
            distinct_tools: l.snapshot.distinct_tools,
            transcript: l.snapshot.transcript,
            messages,
            sidecar_session_id,
            total_tokens: None,
            agent_mode,
        }
    }))
}

/// Sidecar session metadata extracted for display
struct SidecarSessionMeta {
    status: Option<String>,
    title: Option<String>,
}

/// Get metadata from the linked sidecar session for an AI session.
/// Returns status and title extracted from the sidecar session's state.md.
fn get_sidecar_session_meta(session_path: &Path) -> SidecarSessionMeta {
    // Read the sidecar session ID from the companion file
    let sidecar_meta_path = session_path.with_extension("sidecar");
    if !sidecar_meta_path.exists() {
        return SidecarSessionMeta {
            status: None,
            title: None,
        };
    }

    let sidecar_session_id = match std::fs::read_to_string(&sidecar_meta_path) {
        Ok(id) => id.trim().to_string(),
        Err(_) => {
            return SidecarSessionMeta {
                status: None,
                title: None,
            }
        }
    };

    // Get the sidecar sessions directory
    let sessions_dir = std::env::var("VT_SESSION_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".golish")
                .join("sessions")
        });

    // Read the state.md file from the sidecar session
    let state_path = sessions_dir.join(&sidecar_session_id).join("state.md");
    if !state_path.exists() {
        return SidecarSessionMeta {
            status: None,
            title: None,
        };
    }

    let content = match std::fs::read_to_string(&state_path) {
        Ok(c) => c,
        Err(_) => {
            return SidecarSessionMeta {
                status: None,
                title: None,
            }
        }
    };

    // Parse YAML frontmatter to extract status and title
    if !content.starts_with("---\n") {
        return SidecarSessionMeta {
            status: None,
            title: None,
        };
    }

    let rest = &content[4..]; // Skip opening "---\n"
    let end_idx = match rest.find("\n---") {
        Some(idx) => idx,
        None => {
            return SidecarSessionMeta {
                status: None,
                title: None,
            }
        }
    };
    let yaml_content = &rest[..end_idx];

    let mut status = None;
    let mut title = None;

    // Simple extraction of fields
    for line in yaml_content.lines() {
        if line.starts_with("status:") {
            status = Some(line.trim_start_matches("status:").trim().to_string());
        } else if line.starts_with("title:") {
            title = Some(line.trim_start_matches("title:").trim().to_string());
        }
    }

    SidecarSessionMeta { status, title }
}


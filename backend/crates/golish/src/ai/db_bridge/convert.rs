//! Status / agent-type conversions between `golish-db` models and the
//! `golish-agent-kit::db_traits` view enums. Extracted verbatim from
//! `db_bridge.rs`; shared by the per-domain inherent impls.

use golish_agent_kit::db_traits::*;

pub(super) fn convert_dispatch_status_back(
    s: DispatchStatus,
) -> golish_db::models::SubAgentDispatchStatus {
    match s {
        DispatchStatus::Running => golish_db::models::SubAgentDispatchStatus::Running,
        DispatchStatus::Completed => golish_db::models::SubAgentDispatchStatus::Completed,
        DispatchStatus::Failed => golish_db::models::SubAgentDispatchStatus::Failed,
        DispatchStatus::Cancelled => golish_db::models::SubAgentDispatchStatus::Cancelled,
    }
}

pub(super) fn convert_task_status(s: golish_db::models::TaskStatus) -> TaskStatus {
    match s {
        golish_db::models::TaskStatus::Created => TaskStatus::Created,
        golish_db::models::TaskStatus::Running => TaskStatus::Running,
        golish_db::models::TaskStatus::Waiting => TaskStatus::Waiting,
        golish_db::models::TaskStatus::Finished => TaskStatus::Finished,
        golish_db::models::TaskStatus::Failed => TaskStatus::Failed,
    }
}

pub(super) fn convert_task_status_back(s: TaskStatus) -> golish_db::models::TaskStatus {
    match s {
        TaskStatus::Created => golish_db::models::TaskStatus::Created,
        TaskStatus::Running => golish_db::models::TaskStatus::Running,
        TaskStatus::Waiting => golish_db::models::TaskStatus::Waiting,
        TaskStatus::Finished => golish_db::models::TaskStatus::Finished,
        TaskStatus::Failed => golish_db::models::TaskStatus::Failed,
    }
}

pub(super) fn convert_subtask_status(s: golish_db::models::SubtaskStatus) -> SubtaskStatus {
    match s {
        golish_db::models::SubtaskStatus::Created => SubtaskStatus::Created,
        golish_db::models::SubtaskStatus::Running => SubtaskStatus::Running,
        golish_db::models::SubtaskStatus::Waiting => SubtaskStatus::Waiting,
        golish_db::models::SubtaskStatus::Finished => SubtaskStatus::Finished,
        golish_db::models::SubtaskStatus::Failed => SubtaskStatus::Failed,
    }
}

pub(super) fn convert_subtask_status_back(s: SubtaskStatus) -> golish_db::models::SubtaskStatus {
    match s {
        SubtaskStatus::Created => golish_db::models::SubtaskStatus::Created,
        SubtaskStatus::Running => golish_db::models::SubtaskStatus::Running,
        SubtaskStatus::Waiting => golish_db::models::SubtaskStatus::Waiting,
        SubtaskStatus::Finished => golish_db::models::SubtaskStatus::Finished,
        SubtaskStatus::Failed => golish_db::models::SubtaskStatus::Failed,
    }
}

pub(super) fn convert_agent_type(a: golish_db::models::AgentType) -> AgentType {
    match a {
        golish_db::models::AgentType::Primary => AgentType::Primary,
        golish_db::models::AgentType::Pentester => AgentType::Pentester,
        golish_db::models::AgentType::Coder => AgentType::Coder,
        golish_db::models::AgentType::Searcher => AgentType::Searcher,
        golish_db::models::AgentType::Memorist => AgentType::Memorist,
        golish_db::models::AgentType::Reporter => AgentType::Reporter,
        golish_db::models::AgentType::Adviser => AgentType::Adviser,
        golish_db::models::AgentType::Reflector => AgentType::Reflector,
        golish_db::models::AgentType::Enricher => AgentType::Enricher,
        golish_db::models::AgentType::Installer => AgentType::Installer,
        _ => AgentType::Primary,
    }
}

pub(super) fn convert_agent_type_back(a: AgentType) -> golish_db::models::AgentType {
    match a {
        AgentType::Primary => golish_db::models::AgentType::Primary,
        AgentType::Pentester => golish_db::models::AgentType::Pentester,
        AgentType::Coder => golish_db::models::AgentType::Coder,
        AgentType::Searcher => golish_db::models::AgentType::Searcher,
        AgentType::Memorist => golish_db::models::AgentType::Memorist,
        AgentType::Reporter => golish_db::models::AgentType::Reporter,
        AgentType::Adviser => golish_db::models::AgentType::Adviser,
        AgentType::Reflector => golish_db::models::AgentType::Reflector,
        AgentType::Enricher => golish_db::models::AgentType::Enricher,
        AgentType::Installer => golish_db::models::AgentType::Installer,
    }
}

pub(super) fn convert_plan_status(s: golish_db::models::PlanStatus) -> PlanStatus {
    match s {
        golish_db::models::PlanStatus::Planning => PlanStatus::Planning,
        golish_db::models::PlanStatus::InProgress => PlanStatus::InProgress,
        golish_db::models::PlanStatus::Paused => PlanStatus::Paused,
        golish_db::models::PlanStatus::Completed => PlanStatus::Completed,
        golish_db::models::PlanStatus::Failed => PlanStatus::Failed,
        golish_db::models::PlanStatus::Cancelled => PlanStatus::Cancelled,
    }
}

pub(super) fn convert_plan_status_back(s: PlanStatus) -> golish_db::models::PlanStatus {
    match s {
        PlanStatus::Planning => golish_db::models::PlanStatus::Planning,
        PlanStatus::InProgress => golish_db::models::PlanStatus::InProgress,
        PlanStatus::Paused => golish_db::models::PlanStatus::Paused,
        PlanStatus::Completed => golish_db::models::PlanStatus::Completed,
        PlanStatus::Failed => golish_db::models::PlanStatus::Failed,
        PlanStatus::Cancelled => golish_db::models::PlanStatus::Cancelled,
    }
}

//! Shared GUI/CLI task-operation setup.

use std::{collections::HashSet, sync::Arc};

use anyhow::{Context, Result};
use golish_agent_bridge::{
    bridge_executor::BridgeAgentExecutor, AgentBridge, TopLevelRequestLease,
};
use golish_agent_kit::{
    db_traits::{
        CliRuntimeScope, DbRepoProvider, ProjectScopeRegistration, RuntimeMemoryRepository,
        StageForkCreate,
    },
    harness::{ContinuityAdoptionPlan, StageKind},
    task_orchestrator::TaskOrchestrator,
};
use golish_db::{models::NewSession, repo::sessions};
use sqlx::PgPool;
use uuid::Uuid;

use super::db_bridge::GolishDbRepoProvider;

/// Adapter-neutral operation settings applied to every orchestrator before it
/// can create or resume durable work. GUI and CLI may collect these values
/// differently, but they must pass through this single configuration surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskOperationConfig {
    pub profile_override: Option<String>,
    pub stage_allowlist: Option<HashSet<StageKind>>,
    pub harness_org_id: Option<Uuid>,
    pub include_subsidiaries: bool,
    pub subsidiary_threshold: u8,
    pub cli_runtime_scope: Option<CliRuntimeScope>,
    /// `Some(true)` only when this fresh typed launch supplied exact target
    /// intake; `Some(false)` for confirmed-organization-only intake; `None` for
    /// interactive/unconfirmed adapters. Exact resume may restore a persisted
    /// `Some` value through this same config.
    pub current_invocation_target_authority: Option<bool>,
    pub continuity_adoption: Option<ContinuityAdoptionPlan>,
    pub stage_fork: Option<StageForkCreate>,
}

impl Default for TaskOperationConfig {
    fn default() -> Self {
        Self {
            profile_override: None,
            stage_allowlist: None,
            harness_org_id: None,
            include_subsidiaries: false,
            subsidiary_threshold: 51,
            cli_runtime_scope: None,
            current_invocation_target_authority: None,
            continuity_adoption: None,
            stage_fork: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FreshOperationEntry {
    FullProfile,
    StageSlice {
        entry_stage: StageKind,
        allowlist: HashSet<StageKind>,
    },
}

/// Scope expansion policy carried by the fresh-operation launch itself.  It is
/// deliberately separate from the adapter-facing controls so GUI and CLI
/// cannot apply different defaults while building the orchestrator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubsidiaryScopePolicy {
    pub include_subsidiaries: bool,
    pub ownership_threshold_percent: u8,
}

impl Default for SubsidiaryScopePolicy {
    fn default() -> Self {
        Self {
            include_subsidiaries: false,
            ownership_threshold_percent: 51,
        }
    }
}

/// Trusted scope material collected for this fresh invocation.
///
/// A company name that exists only in GUI prompt text is an engagement subject,
/// so `UnconfirmedSubject` intentionally has no organization id, exact targets,
/// or runtime-scope authority fields. A headless adapter may deliberately
/// confirm an explicit company intake after it has get-or-created the exact
/// organization row; that narrow path uses `ConfirmedOrganizationIntake` and
/// still carries no target authority. Exact targets can enter only through
/// `ConfirmedTargetIntake`, whose values are supplied by the current invocation
/// rather than inferred from a company label, historical organization, or
/// provider data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FreshOperationScope {
    UnconfirmedSubject {
        label: String,
    },
    ConfirmedTargetIntake {
        subject_label: Option<String>,
        current_invocation_targets: Vec<String>,
        organization_id: Option<Uuid>,
        runtime_scope: Option<CliRuntimeScope>,
    },
    ConfirmedOrganizationIntake {
        subject_label: String,
        organization_id: Uuid,
        runtime_scope: Option<CliRuntimeScope>,
    },
}

impl FreshOperationScope {
    pub fn unconfirmed_subject(label: impl Into<String>) -> Result<Self> {
        let scope = Self::UnconfirmedSubject {
            label: label.into(),
        };
        scope.validate(&SubsidiaryScopePolicy::default())?;
        Ok(scope)
    }

    pub fn confirmed_target_intake(
        subject_label: Option<String>,
        current_invocation_targets: Vec<String>,
        organization_id: Option<Uuid>,
        runtime_scope: Option<CliRuntimeScope>,
        policy: &SubsidiaryScopePolicy,
    ) -> Result<Self> {
        let scope = Self::ConfirmedTargetIntake {
            subject_label,
            current_invocation_targets,
            organization_id,
            runtime_scope,
        };
        scope.validate(policy)?;
        Ok(scope)
    }

    pub fn confirmed_organization_intake(
        subject_label: impl Into<String>,
        organization_id: Uuid,
        runtime_scope: Option<CliRuntimeScope>,
        policy: &SubsidiaryScopePolicy,
    ) -> Result<Self> {
        let scope = Self::ConfirmedOrganizationIntake {
            subject_label: subject_label.into(),
            organization_id,
            runtime_scope,
        };
        scope.validate(policy)?;
        Ok(scope)
    }

    fn validate(&self, policy: &SubsidiaryScopePolicy) -> Result<()> {
        match self {
            Self::UnconfirmedSubject { label } => {
                anyhow::ensure!(
                    !label.trim().is_empty(),
                    "fresh operation subject label cannot be empty"
                );
            }
            Self::ConfirmedTargetIntake {
                subject_label,
                current_invocation_targets,
                organization_id,
                runtime_scope,
            } => {
                if let Some(label) = subject_label {
                    anyhow::ensure!(
                        !label.trim().is_empty(),
                        "confirmed target subject label cannot be empty"
                    );
                }
                anyhow::ensure!(
                    !current_invocation_targets.is_empty(),
                    "confirmed target intake requires a current-invocation exact target"
                );
                validate_current_invocation_exact_targets(current_invocation_targets)?;
                validate_runtime_scope_authority(*organization_id, runtime_scope.as_ref(), policy)?;
            }
            Self::ConfirmedOrganizationIntake {
                subject_label,
                organization_id,
                runtime_scope,
            } => {
                anyhow::ensure!(
                    !subject_label.trim().is_empty(),
                    "confirmed organization subject label cannot be empty"
                );
                validate_runtime_scope_authority(
                    Some(*organization_id),
                    runtime_scope.as_ref(),
                    policy,
                )?;
            }
        }
        Ok(())
    }
}

pub fn validate_current_invocation_exact_targets(targets: &[String]) -> Result<()> {
    for (index, target) in targets.iter().enumerate() {
        anyhow::ensure!(
            is_canonical_exact_target_shape(target),
            "current-invocation target at index {index} is not a canonical exact target shape"
        );
    }
    Ok(())
}

/// Validate only explicit target syntax; this never derives a target from an
/// objective or company label.  Values remain unchanged for the downstream DB
/// intake, but malformed/prose inputs never become launch authority.
fn is_canonical_exact_target_shape(value: &str) -> bool {
    if value.is_empty()
        || value != value.trim()
        || value
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control())
    {
        return false;
    }

    let lower = value.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return golish_pentest_domain::canonical_web_origin(value)
            .is_some_and(|origin| valid_url_host(&origin.host));
    }
    if value.contains("://") {
        return false;
    }
    if value.parse::<std::net::IpAddr>().is_ok() {
        return true;
    }
    if let Some((address, prefix)) = value.split_once('/') {
        let Ok(address) = address.parse::<std::net::IpAddr>() else {
            return false;
        };
        let Ok(prefix) = prefix.parse::<u8>() else {
            return false;
        };
        return match address {
            std::net::IpAddr::V4(_) => prefix <= 32,
            std::net::IpAddr::V6(_) => prefix <= 128,
        };
    }
    if let Some(base) = value.strip_prefix("*.") {
        return base.contains('.') && valid_dns_hostname(base);
    }
    valid_dns_hostname(value)
}

fn valid_url_host(host: &str) -> bool {
    let unbracketed = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host);
    unbracketed.parse::<std::net::IpAddr>().is_ok() || valid_dns_hostname(unbracketed)
}

fn valid_dns_hostname(value: &str) -> bool {
    let hostname = value.strip_suffix('.').unwrap_or(value);
    if hostname.is_empty() || hostname.len() > 253 || !hostname.is_ascii() {
        return false;
    }
    if hostname != "localhost" && !hostname.contains('.') {
        return false;
    }
    hostname.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    })
}

fn validate_runtime_scope_authority(
    organization_id: Option<Uuid>,
    runtime_scope: Option<&CliRuntimeScope>,
    policy: &SubsidiaryScopePolicy,
) -> Result<()> {
    let Some(runtime_scope) = runtime_scope else {
        return Ok(());
    };
    let organization_id = organization_id
        .context("fresh operation runtime scope cannot exist without organization authority")?;
    anyhow::ensure!(
        runtime_scope.root_organization_id == organization_id,
        "fresh operation runtime scope root does not match organization authority"
    );
    anyhow::ensure!(
        runtime_scope.include_subsidiaries == policy.include_subsidiaries
            && runtime_scope.subsidiary_threshold == policy.ownership_threshold_percent,
        "fresh operation runtime scope does not match subsidiary policy"
    );
    let root_unit = runtime_scope
        .units
        .first()
        .context("fresh operation runtime scope has no root unit")?;
    anyhow::ensure!(
        root_unit.organization_id == runtime_scope.root_organization_id,
        "fresh operation runtime scope first unit is not its root"
    );
    Ok(())
}

/// One adapter-neutral launch contract for every fresh operation.  Execution
/// boundary remains in `entry`; security-relevant authority is projected from
/// `scope` and can never be reconstructed from `objective` text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreshTaskOperationLaunch {
    pub objective: String,
    pub profile_id: String,
    pub entry: FreshOperationEntry,
    pub scope: FreshOperationScope,
    pub subsidiary_policy: SubsidiaryScopePolicy,
    pub continuity_adoption: Option<ContinuityAdoptionPlan>,
}

/// Trusted launch contract for a new operation that adopts an exact source
/// operation prefix and executes one post-Scoping stage slice. It is separate
/// from continuity adoption: the DB repository, not a global completion
/// ledger, validates and freezes every adopted input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageForkTaskOperationLaunch {
    pub objective: String,
    pub profile_id: String,
    pub source_operation_id: Uuid,
    pub source_scope_snapshot_id: Uuid,
    pub entry_stage: StageKind,
    pub terminal_stage: StageKind,
    pub allowlist: HashSet<StageKind>,
    pub adopted_stage_kinds: Vec<StageKind>,
    pub scope: FreshOperationScope,
    pub subsidiary_policy: SubsidiaryScopePolicy,
}

impl StageForkTaskOperationLaunch {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        objective: impl Into<String>,
        profile_id: impl Into<String>,
        source_operation_id: Uuid,
        source_scope_snapshot_id: Uuid,
        entry_stage: StageKind,
        terminal_stage: StageKind,
        allowlist: HashSet<StageKind>,
        adopted_stage_kinds: Vec<StageKind>,
        scope: FreshOperationScope,
        subsidiary_policy: SubsidiaryScopePolicy,
    ) -> Result<Self> {
        let launch = Self {
            objective: objective.into(),
            profile_id: profile_id.into(),
            source_operation_id,
            source_scope_snapshot_id,
            entry_stage,
            terminal_stage,
            allowlist,
            adopted_stage_kinds,
            scope,
            subsidiary_policy,
        };
        launch.validate()?;
        Ok(launch)
    }

    fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            !self.objective.trim().is_empty(),
            "stage fork objective cannot be empty"
        );
        anyhow::ensure!(
            !self.source_operation_id.is_nil(),
            "stage fork source operation is nil"
        );
        anyhow::ensure!(
            !self.source_scope_snapshot_id.is_nil(),
            "stage fork source scope is nil"
        );
        anyhow::ensure!(
            self.entry_stage != StageKind::Scoping,
            "stage fork cannot execute Scoping"
        );
        anyhow::ensure!(
            self.allowlist.contains(&self.entry_stage),
            "stage fork slice omits its entry"
        );
        anyhow::ensure!(
            self.allowlist.contains(&self.terminal_stage),
            "stage fork slice omits its terminal"
        );
        anyhow::ensure!(
            self.adopted_stage_kinds.first() == Some(&StageKind::Scoping)
                && !self.adopted_stage_kinds.contains(&self.entry_stage),
            "stage fork must adopt a strict prefix beginning at Scoping"
        );
        anyhow::ensure!(
            golish_agent_kit::harness::load_embedded_profile(&self.profile_id)
                .context("load stage fork harness profile")?
                .is_some(),
            "unknown harness profile: {}",
            self.profile_id
        );
        anyhow::ensure!(
            matches!(
                &self.scope,
                FreshOperationScope::ConfirmedTargetIntake {
                    runtime_scope: Some(_),
                    ..
                } | FreshOperationScope::ConfirmedOrganizationIntake {
                    runtime_scope: Some(_),
                    ..
                }
            ),
            "stage fork scope must contain the frozen selected source operation scope"
        );
        self.scope.validate(&self.subsidiary_policy)
    }

    fn task_operation_config(&self) -> Result<TaskOperationConfig> {
        self.validate()?;
        let (harness_org_id, cli_runtime_scope, current_invocation_target_authority) =
            match &self.scope {
                FreshOperationScope::ConfirmedTargetIntake {
                    organization_id,
                    runtime_scope,
                    ..
                } => (*organization_id, runtime_scope.clone(), Some(true)),
                FreshOperationScope::ConfirmedOrganizationIntake {
                    organization_id,
                    runtime_scope,
                    ..
                } => {
                    // Unlike an ordinary company-only fresh launch, a stage
                    // fork freezes the current DB Targets atomically with its
                    // lineage manifest before any active stage dispatch.
                    (Some(*organization_id), runtime_scope.clone(), Some(true))
                }
                FreshOperationScope::UnconfirmedSubject { .. } => unreachable!("validated above"),
            };
        Ok(TaskOperationConfig {
            profile_override: Some(self.profile_id.clone()),
            stage_allowlist: Some(self.allowlist.clone()),
            harness_org_id,
            include_subsidiaries: self.subsidiary_policy.include_subsidiaries,
            subsidiary_threshold: self.subsidiary_policy.ownership_threshold_percent,
            cli_runtime_scope,
            current_invocation_target_authority,
            continuity_adoption: None,
            stage_fork: Some(StageForkCreate {
                source_operation_id: self.source_operation_id,
                source_scope_snapshot_id: self.source_scope_snapshot_id,
                entry_stage: self.entry_stage.as_str().to_string(),
                terminal_stage: self.terminal_stage.as_str().to_string(),
                adopted_stage_kinds: self
                    .adopted_stage_kinds
                    .iter()
                    .map(|stage| stage.as_str().to_string())
                    .collect(),
                operation_contract_adoption: None,
            }),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FreshLaunchAuthorityScope {
    UnconfirmedSubject { label: String },
    ConfirmedTargetIntake { subject_label: Option<String> },
    ConfirmedOrganizationIntake { subject_label: String },
}

/// Adapter-neutral semantic/authority projection.  It intentionally records
/// the common start stage while leaving FullProfile-vs-StageSlice execution
/// boundaries in [`FreshTaskOperationLaunch::entry`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreshLaunchAuthorityProjection {
    pub objective: String,
    pub profile_id: String,
    pub start_stage: StageKind,
    pub scope: FreshLaunchAuthorityScope,
    pub current_invocation_targets: Vec<String>,
    pub organization_id: Option<Uuid>,
    pub runtime_scope: Option<CliRuntimeScope>,
    pub subsidiary_policy: SubsidiaryScopePolicy,
}

impl FreshTaskOperationLaunch {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        objective: impl Into<String>,
        profile_id: impl Into<String>,
        entry: FreshOperationEntry,
        scope: FreshOperationScope,
        subsidiary_policy: SubsidiaryScopePolicy,
        continuity_adoption: Option<ContinuityAdoptionPlan>,
    ) -> Result<Self> {
        let launch = Self {
            objective: objective.into(),
            profile_id: profile_id.into(),
            entry,
            scope,
            subsidiary_policy,
            continuity_adoption,
        };
        launch.validate()?;
        Ok(launch)
    }

    pub fn normalized_authority_projection(&self) -> Result<FreshLaunchAuthorityProjection> {
        self.validate()?;
        let (scope, current_invocation_targets, organization_id, runtime_scope) = match &self.scope
        {
            FreshOperationScope::UnconfirmedSubject { label } => (
                FreshLaunchAuthorityScope::UnconfirmedSubject {
                    label: label.clone(),
                },
                Vec::new(),
                None,
                None,
            ),
            FreshOperationScope::ConfirmedTargetIntake {
                subject_label,
                current_invocation_targets,
                organization_id,
                runtime_scope,
            } => (
                FreshLaunchAuthorityScope::ConfirmedTargetIntake {
                    subject_label: subject_label.clone(),
                },
                current_invocation_targets.clone(),
                *organization_id,
                runtime_scope.clone(),
            ),
            FreshOperationScope::ConfirmedOrganizationIntake {
                subject_label,
                organization_id,
                runtime_scope,
            } => (
                FreshLaunchAuthorityScope::ConfirmedOrganizationIntake {
                    subject_label: subject_label.clone(),
                },
                Vec::new(),
                Some(*organization_id),
                runtime_scope.clone(),
            ),
        };
        Ok(FreshLaunchAuthorityProjection {
            objective: self.objective.clone(),
            profile_id: self.profile_id.clone(),
            start_stage: match &self.entry {
                FreshOperationEntry::FullProfile => StageKind::Scoping,
                FreshOperationEntry::StageSlice { entry_stage, .. } => *entry_stage,
            },
            scope,
            current_invocation_targets,
            organization_id,
            runtime_scope,
            subsidiary_policy: self.subsidiary_policy.clone(),
        })
    }

    fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            !self.objective.trim().is_empty(),
            "fresh operation objective cannot be empty"
        );
        anyhow::ensure!(
            !self.profile_id.trim().is_empty(),
            "fresh operation profile cannot be empty"
        );
        anyhow::ensure!(
            golish_agent_kit::harness::load_embedded_profile(&self.profile_id)
                .context("load fresh operation harness profile")?
                .is_some(),
            "unknown harness profile: {}",
            self.profile_id
        );
        if let FreshOperationEntry::StageSlice {
            entry_stage,
            allowlist,
        } = &self.entry
        {
            anyhow::ensure!(
                allowlist.contains(entry_stage),
                "fresh operation stage slice does not include its entry stage"
            );
            anyhow::ensure!(
                self.continuity_adoption.is_none(),
                "stage slices cannot apply a full-profile continuity adoption plan"
            );
        }
        self.scope.validate(&self.subsidiary_policy)
    }

    fn task_operation_config(&self) -> Result<TaskOperationConfig> {
        self.validate()?;
        let (harness_org_id, cli_runtime_scope, current_invocation_target_authority) =
            match &self.scope {
                FreshOperationScope::UnconfirmedSubject { .. } => (None, None, None),
                FreshOperationScope::ConfirmedTargetIntake {
                    organization_id,
                    runtime_scope,
                    ..
                } => (*organization_id, runtime_scope.clone(), Some(true)),
                FreshOperationScope::ConfirmedOrganizationIntake {
                    organization_id,
                    runtime_scope,
                    ..
                } => (Some(*organization_id), runtime_scope.clone(), Some(false)),
            };
        let config = TaskOperationConfig {
            profile_override: Some(self.profile_id.clone()),
            stage_allowlist: match &self.entry {
                FreshOperationEntry::FullProfile => None,
                FreshOperationEntry::StageSlice { allowlist, .. } => Some(allowlist.clone()),
            },
            harness_org_id,
            include_subsidiaries: self.subsidiary_policy.include_subsidiaries,
            subsidiary_threshold: self.subsidiary_policy.ownership_threshold_percent,
            cli_runtime_scope,
            current_invocation_target_authority,
            continuity_adoption: self.continuity_adoption.clone(),
            stage_fork: None,
        };
        self.validate_config_consistency(&config)?;
        Ok(config)
    }

    fn validate_config_consistency(&self, config: &TaskOperationConfig) -> Result<()> {
        anyhow::ensure!(
            config.profile_override.as_deref() == Some(self.profile_id.as_str()),
            "fresh operation config profile diverges from typed launch"
        );
        let expected_allowlist = match &self.entry {
            FreshOperationEntry::FullProfile => None,
            FreshOperationEntry::StageSlice { allowlist, .. } => Some(allowlist),
        };
        anyhow::ensure!(
            config.stage_allowlist.as_ref() == expected_allowlist,
            "fresh operation config entry boundary diverges from typed launch"
        );
        anyhow::ensure!(
            config.include_subsidiaries == self.subsidiary_policy.include_subsidiaries
                && config.subsidiary_threshold
                    == self.subsidiary_policy.ownership_threshold_percent,
            "fresh operation config subsidiary policy diverges from typed launch"
        );
        anyhow::ensure!(
            config.continuity_adoption == self.continuity_adoption,
            "fresh operation config continuity adoption diverges from typed launch"
        );
        let (expected_org_id, expected_runtime_scope, expected_target_authority) = match &self.scope
        {
            FreshOperationScope::UnconfirmedSubject { .. } => (None, None, None),
            FreshOperationScope::ConfirmedTargetIntake {
                organization_id,
                runtime_scope,
                ..
            } => (*organization_id, runtime_scope.as_ref(), Some(true)),
            FreshOperationScope::ConfirmedOrganizationIntake {
                organization_id,
                runtime_scope,
                ..
            } => (Some(*organization_id), runtime_scope.as_ref(), Some(false)),
        };
        anyhow::ensure!(
            config.harness_org_id == expected_org_id
                && config.cli_runtime_scope.as_ref() == expected_runtime_scope
                && config.current_invocation_target_authority == expected_target_authority,
            "fresh operation config scope authority diverges from typed launch"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreshTaskOperationOutcome {
    pub session_id: Uuid,
    pub response: String,
}

/// Fully prepared shared operation context. No tool can dispatch while this is
/// being built: the executor is returned to the caller only after the DB
/// session owner, tracker owner, repository views and canonical project scope
/// are all ready.
pub struct PreparedTaskOperation {
    bridge: Arc<AgentBridge>,
    request: TopLevelRequestLease,
    executor: BridgeAgentExecutor,
    db_repo: Arc<dyn DbRepoProvider>,
    runtime_repo: Arc<dyn RuntimeMemoryRepository>,
    session_id: Uuid,
    chat_session_key: String,
}

/// Prepare the common operation context used by GUI Task/Profile and headless
/// CLI stage runs. The workspace identity always comes from the configured
/// bridge, so an adapter cannot register a different path than the one tools
/// actually use.
pub async fn prepare_task_operation(
    bridge: Arc<AgentBridge>,
    db_pool: Arc<PgPool>,
    chat_session_key: &str,
    task_input: &str,
    request: TopLevelRequestLease,
) -> Result<PreparedTaskOperation> {
    let cleanup_bridge = bridge.clone();
    let cleanup_request = request.clone();
    let prepared =
        prepare_task_operation_inner(bridge, db_pool, chat_session_key, task_input, request).await;

    if prepared.is_err() {
        if let Err(cleanup_error) = cleanup_bridge
            .clear_top_level_request_state(&cleanup_request)
            .await
        {
            tracing::error!(
                target: "harness::task_operation",
                error = %cleanup_error,
                "failed to clear request-local state after operation setup failure"
            );
        }
    }
    prepared
}

async fn prepare_task_operation_inner(
    bridge: Arc<AgentBridge>,
    db_pool: Arc<PgPool>,
    chat_session_key: &str,
    task_input: &str,
    request: TopLevelRequestLease,
) -> Result<PreparedTaskOperation> {
    let session_row = sessions::upsert_by_chat_key(
        &db_pool,
        chat_session_key,
        NewSession {
            title: Some(task_session_title(task_input)),
            workspace_path: None,
            workspace_label: None,
            model: Some(bridge.model_name().to_string()),
            provider: Some(bridge.provider_name().to_string()),
            project_path: None,
        },
    )
    .await
    .context("upsert operation session row (FK precondition for tasks)")?;

    // This ordering is a security contract: runtime tool rows must never see
    // the random tracker UUID that exists before the durable chat-key session.
    bridge.set_tracker_session_uuid(session_row.id);
    let executor = BridgeAgentExecutor::from_request(bridge.clone(), request.clone())
        .context("upgrade owned request into Task execution")?;

    let provider = Arc::new(GolishDbRepoProvider::new(db_pool));
    let db_repo: Arc<dyn DbRepoProvider> = provider.clone();
    let runtime_repo: Arc<dyn RuntimeMemoryRepository> = provider;

    Ok(PreparedTaskOperation {
        bridge,
        request,
        executor,
        db_repo,
        runtime_repo,
        session_id: session_row.id,
        chat_session_key: chat_session_key.to_string(),
    })
}

impl PreparedTaskOperation {
    pub fn session_id(&self) -> Uuid {
        self.session_id
    }

    pub fn db_repo(&self) -> &dyn DbRepoProvider {
        self.db_repo.as_ref()
    }

    pub fn executor(&self) -> &BridgeAgentExecutor {
        &self.executor
    }

    /// Resolve and register the same canonical workspace held by the bridge.
    /// This stays separate from preparation so a GUI continuity prompt can
    /// return without creating a project-scope row.
    pub async fn register_project_scope(&self) -> Result<ProjectScopeRegistration> {
        let workspace = self.bridge.workspace().read().await.clone();
        let (canonical_path, path_sha256) =
            golish_agent_kit::runtime_memory::canonical_workspace_identity(&workspace)
                .map_err(anyhow::Error::new)
                .context("resolve trusted workspace identity for runtime operation")?;
        self.runtime_repo
            .project_scope_register_first_open(&canonical_path, &path_sha256)
            .await
            .map_err(anyhow::Error::new)
            .context("register trusted project scope for runtime operation")
    }

    /// Build the one canonical orchestrator configuration used by every GUI or
    /// CLI adapter. Entry selection (`run`, `run_stage`, `resume`) remains a
    /// caller decision, but the security-relevant context does not.
    pub fn build_orchestrator(&self, config: TaskOperationConfig) -> TaskOrchestrator {
        let mut orchestrator = TaskOrchestrator::new(
            self.db_repo.clone(),
            self.runtime_repo.clone(),
            self.session_id,
            self.bridge.get_or_create_event_tx(),
        );
        orchestrator.set_profile_override(config.profile_override);
        orchestrator.set_chat_session_id(self.chat_session_key.clone());
        orchestrator.set_approval_coordinator(self.bridge.coordinator().cloned());
        orchestrator.set_stage_allowlist(config.stage_allowlist);
        orchestrator.set_harness_org_id(config.harness_org_id);
        orchestrator.set_subsidiary_scope(config.include_subsidiaries, config.subsidiary_threshold);
        orchestrator.set_cli_runtime_scope(config.cli_runtime_scope);
        orchestrator
            .set_current_invocation_target_authority(config.current_invocation_target_authority);
        orchestrator.set_continuity_adoption(config.continuity_adoption);
        orchestrator.set_stage_fork(config.stage_fork);
        orchestrator
    }

    /// Execute a fresh full-profile or stage-slice operation with the same
    /// prepared session/tracker/repository/project-scope context.
    pub async fn run_fresh(
        self,
        launch: FreshTaskOperationLaunch,
    ) -> Result<FreshTaskOperationOutcome> {
        let config = match launch.task_operation_config() {
            Ok(config) => config,
            Err(error) => return self.finish(Err(error)).await,
        };

        let project_scope = match self.register_project_scope().await {
            Ok(scope) => scope,
            Err(error) => return self.finish(Err(error)).await,
        };
        let mut orchestrator = self.build_orchestrator(config);
        let result = match launch.entry {
            FreshOperationEntry::FullProfile => {
                orchestrator
                    .run(&launch.objective, project_scope, self.executor())
                    .await
            }
            FreshOperationEntry::StageSlice { entry_stage, .. } => {
                orchestrator
                    .run_stage(
                        entry_stage,
                        &launch.objective,
                        project_scope,
                        self.executor(),
                    )
                    .await
            }
        };
        let session_id = self.session_id;
        self.finish(result)
            .await
            .map(|response| FreshTaskOperationOutcome {
                session_id,
                response,
            })
    }

    /// Execute a new stage-testing fork through the same orchestrator stage
    /// kernel as ordinary GUI and CLI operations.
    pub async fn run_stage_fork(
        self,
        launch: StageForkTaskOperationLaunch,
    ) -> Result<FreshTaskOperationOutcome> {
        let config = match launch.task_operation_config() {
            Ok(config) => config,
            Err(error) => return self.finish(Err(error)).await,
        };
        let project_scope = match self.register_project_scope().await {
            Ok(scope) => scope,
            Err(error) => return self.finish(Err(error)).await,
        };
        let mut orchestrator = self.build_orchestrator(config);
        let result = orchestrator
            .run_stage(
                launch.entry_stage,
                &launch.objective,
                project_scope,
                self.executor(),
            )
            .await;
        let session_id = self.session_id;
        let response = self.finish(result).await?;
        Ok(FreshTaskOperationOutcome {
            session_id,
            response,
        })
    }

    /// Clear request-scoped harness state without allowing cleanup failure to
    /// hide the primary operation error.
    pub async fn finish<T>(self, result: Result<T>) -> Result<T> {
        let cleanup = self
            .bridge
            .clear_top_level_request_state(&self.request)
            .await;
        match (result, cleanup) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        }
    }
}

/// Stable UTF-8-safe session title shared by GUI and CLI adapters.
pub fn task_session_title(input: &str) -> String {
    truncate_session_title(input, 80)
}

pub(crate) fn truncate_session_title(input: &str, max_bytes: usize) -> String {
    if input.len() <= max_bytes {
        return input.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !input.is_char_boundary(end) {
        end -= 1;
    }
    input[..end].to_string()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use golish_agent_kit::{
        db_traits::{CliRuntimeScope, CliRuntimeScopeUnit},
        harness::StageKind,
    };

    use super::{
        task_session_title, FreshLaunchAuthorityScope, FreshOperationEntry, FreshOperationScope,
        FreshTaskOperationLaunch, StageForkTaskOperationLaunch, SubsidiaryScopePolicy,
        TaskOperationConfig,
    };

    #[test]
    fn explicit_cli_company_confirms_org_without_matching_gui_or_target_authority() {
        let objective = "广州有创网络科技有限公司";
        let organization_id = uuid::Uuid::from_u128(0x4985d1d1843a43fcabda35d03441e7f2);
        let policy = SubsidiaryScopePolicy::default();
        let gui = FreshTaskOperationLaunch::new(
            objective,
            "red_team",
            FreshOperationEntry::FullProfile,
            FreshOperationScope::unconfirmed_subject(objective).expect("valid GUI subject"),
            policy.clone(),
            None,
        )
        .expect("valid GUI launch");
        let cli_allowlist = HashSet::from([
            StageKind::Scoping,
            StageKind::TargetIntel,
            StageKind::ExternalAttackSurface,
            StageKind::Enumeration,
            StageKind::VulnTriage,
            StageKind::AttackCandidate,
        ]);
        let cli = FreshTaskOperationLaunch::new(
            objective,
            "red_team",
            FreshOperationEntry::StageSlice {
                entry_stage: StageKind::Scoping,
                allowlist: cli_allowlist.clone(),
            },
            FreshOperationScope::confirmed_organization_intake(
                objective,
                organization_id,
                None,
                &policy,
            )
            .expect("valid explicit CLI organization"),
            policy.clone(),
            None,
        )
        .expect("valid CLI launch");
        assert_eq!(
            gui.task_operation_config()
                .expect("GUI config")
                .current_invocation_target_authority,
            None
        );
        assert_eq!(
            cli.task_operation_config()
                .expect("CLI config")
                .current_invocation_target_authority,
            Some(false)
        );

        assert!(matches!(&gui.entry, FreshOperationEntry::FullProfile));
        assert!(matches!(
            &cli.entry,
            FreshOperationEntry::StageSlice {
                entry_stage: StageKind::Scoping,
                allowlist,
            } if allowlist == &cli_allowlist
        ));

        let gui_authority = gui
            .normalized_authority_projection()
            .expect("valid GUI authority");
        let cli_authority = cli
            .normalized_authority_projection()
            .expect("valid CLI authority");
        assert_ne!(gui_authority.scope, cli_authority.scope);
        assert_eq!(gui_authority.profile_id, cli_authority.profile_id);
        assert_eq!(gui_authority.start_stage, cli_authority.start_stage);
        assert_eq!(gui_authority.subsidiary_policy, policy);
        assert_eq!(
            gui_authority.subsidiary_policy,
            cli_authority.subsidiary_policy
        );
        assert_eq!(
            gui_authority.scope,
            FreshLaunchAuthorityScope::UnconfirmedSubject {
                label: objective.to_string(),
            }
        );
        assert_eq!(
            cli_authority.scope,
            FreshLaunchAuthorityScope::ConfirmedOrganizationIntake {
                subject_label: objective.to_string(),
            }
        );
        assert!(gui_authority.current_invocation_targets.is_empty());
        assert!(cli_authority.current_invocation_targets.is_empty());
        assert_eq!(gui_authority.organization_id, None);
        assert_eq!(cli_authority.organization_id, Some(organization_id));
        assert_eq!(gui_authority.runtime_scope, None);
        assert_eq!(cli_authority.runtime_scope, None);
    }

    #[test]
    fn company_only_launch_maps_to_config_without_scope_authority() {
        let objective = "广州有创网络科技有限公司";
        let launch = FreshTaskOperationLaunch::new(
            objective,
            "red_team",
            FreshOperationEntry::FullProfile,
            FreshOperationScope::unconfirmed_subject(objective).expect("valid subject"),
            SubsidiaryScopePolicy::default(),
            None,
        )
        .expect("valid company-only launch");

        let config = launch
            .task_operation_config()
            .expect("company-only config maps safely");
        assert_eq!(config.profile_override.as_deref(), Some("red_team"));
        assert_eq!(config.harness_org_id, None);
        assert_eq!(config.cli_runtime_scope, None);

        let mut injected = config;
        injected.harness_org_id = Some(uuid::Uuid::new_v4());
        let error = launch
            .validate_config_consistency(&injected)
            .expect_err("injected organization authority must fail closed");
        assert!(error.to_string().contains("scope authority diverges"));
    }

    #[test]
    fn confirmed_target_intake_requires_current_invocation_exact_targets() {
        let error = FreshOperationScope::confirmed_target_intake(
            Some("广州有创网络科技有限公司".to_string()),
            Vec::new(),
            None,
            None,
            &SubsidiaryScopePolicy::default(),
        )
        .expect_err("history/provider data cannot substitute for invocation targets");
        assert!(error
            .to_string()
            .contains("current-invocation exact target"));
    }

    #[test]
    fn confirmed_target_launch_marks_current_invocation_authority() {
        let policy = SubsidiaryScopePolicy::default();
        let launch = FreshTaskOperationLaunch::new(
            "loopback fixture",
            "red_team",
            FreshOperationEntry::StageSlice {
                entry_stage: StageKind::Scoping,
                allowlist: HashSet::from([
                    StageKind::Scoping,
                    StageKind::TargetIntel,
                    StageKind::ExternalAttackSurface,
                ]),
            },
            FreshOperationScope::confirmed_target_intake(
                Some("fixture org".to_string()),
                vec!["http://127.0.0.1:18080".to_string()],
                None,
                None,
                &policy,
            )
            .expect("valid current target"),
            policy,
            None,
        )
        .expect("valid target launch");

        assert_eq!(
            launch
                .task_operation_config()
                .expect("target config")
                .current_invocation_target_authority,
            Some(true)
        );
    }

    #[test]
    fn confirmed_target_intake_rejects_prose_and_malformed_shapes() {
        let policy = SubsidiaryScopePolicy::default();
        for invalid in [
            "scan this company",
            "广州有创网络科技有限公司",
            "https://",
            "10.0.0.1/33",
            "2001:db8::1/129",
            "*.bad label.example",
        ] {
            let result = FreshOperationScope::confirmed_target_intake(
                None,
                vec![invalid.to_string()],
                None,
                None,
                &policy,
            );
            assert!(
                result.is_err(),
                "invalid target shape was accepted: {invalid}"
            );
        }

        for valid in [
            "https://example.com:8443/path?q=1",
            "127.0.0.1",
            "2001:db8::1",
            "10.0.0.0/24",
            "2001:db8::/64",
            "example.com",
            "localhost",
            "*.example.com",
        ] {
            FreshOperationScope::confirmed_target_intake(
                None,
                vec![valid.to_string()],
                None,
                None,
                &policy,
            )
            .unwrap_or_else(|error| panic!("valid target shape was rejected: {valid}: {error}"));
        }
    }

    #[test]
    fn fresh_launch_rejects_unknown_profile_instead_of_falling_back() {
        let result = FreshTaskOperationLaunch::new(
            "广州有创网络科技有限公司",
            "red_team_typo",
            FreshOperationEntry::FullProfile,
            FreshOperationScope::unconfirmed_subject("广州有创网络科技有限公司")
                .expect("valid subject"),
            SubsidiaryScopePolicy::default(),
            None,
        );

        let error = result.expect_err("an unknown profile must fail closed before execution");
        assert!(error.to_string().contains("unknown harness profile"));
    }

    #[test]
    fn task_operation_config_keeps_adapter_independent_semantics_together() {
        let allowlist = HashSet::from([StageKind::Scoping, StageKind::AttackCandidate]);
        let config = TaskOperationConfig {
            profile_override: Some("red_team".to_string()),
            stage_allowlist: Some(allowlist.clone()),
            harness_org_id: None,
            include_subsidiaries: false,
            subsidiary_threshold: 51,
            cli_runtime_scope: None,
            current_invocation_target_authority: None,
            continuity_adoption: None,
            stage_fork: None,
        };

        assert_eq!(config.profile_override.as_deref(), Some("red_team"));
        assert_eq!(config.stage_allowlist, Some(allowlist));
        assert_eq!(config.subsidiary_threshold, 51);
    }

    #[test]
    fn stage_fork_launch_projects_exact_lineage_into_shared_orchestrator_config() {
        let source_operation_id = uuid::Uuid::new_v4();
        let source_scope_snapshot_id = uuid::Uuid::new_v4();
        let organization_id = uuid::Uuid::new_v4();
        let runtime_scope = CliRuntimeScope {
            root_organization_id: organization_id,
            include_subsidiaries: false,
            subsidiary_threshold: 51,
            units: vec![CliRuntimeScopeUnit {
                organization_id,
                parent_organization_id: None,
                organization_name: "fixture org".to_string(),
                depth: 0,
                ordinal: 0,
                ownership_percent: None,
                approval_source: serde_json::json!({
                    "kind": "stage_fork_source_scope",
                    "source_operation_id": source_operation_id,
                    "source_scope_snapshot_id": source_scope_snapshot_id,
                }),
            }],
        };
        let launch = StageForkTaskOperationLaunch::new(
            "rerun candidate only",
            "pentest",
            source_operation_id,
            source_scope_snapshot_id,
            StageKind::AttackCandidate,
            StageKind::AttackCandidate,
            HashSet::from([StageKind::AttackCandidate]),
            vec![
                StageKind::Scoping,
                StageKind::TargetIntel,
                StageKind::ExternalAttackSurface,
                StageKind::Enumeration,
                StageKind::VulnTriage,
            ],
            FreshOperationScope::confirmed_organization_intake(
                "fixture org",
                organization_id,
                Some(runtime_scope),
                &SubsidiaryScopePolicy::default(),
            )
            .expect("source scope"),
            SubsidiaryScopePolicy::default(),
        )
        .expect("stage fork launch");

        let config = launch.task_operation_config().expect("fork config");
        assert_eq!(
            config.stage_allowlist,
            Some(HashSet::from([StageKind::AttackCandidate]))
        );
        assert!(config.continuity_adoption.is_none());
        let fork = config.stage_fork.expect("typed DB lineage");
        assert_eq!(fork.source_operation_id, source_operation_id);
        assert_eq!(fork.source_scope_snapshot_id, source_scope_snapshot_id);
        assert_eq!(fork.entry_stage, "attack_candidate");
        assert_eq!(fork.terminal_stage, "attack_candidate");
        assert_eq!(
            fork.adopted_stage_kinds.last().map(String::as_str),
            Some("vuln_triage")
        );
    }

    #[test]
    fn shared_session_title_is_utf8_safe() {
        let title = task_session_title(&"广".repeat(40));
        assert!(title.len() <= 80);
        assert!(title.is_char_boundary(title.len()));
    }
}

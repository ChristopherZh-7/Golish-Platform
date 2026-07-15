use std::collections::HashSet;

use golish_agent_app::ai::task_operation::{
    FreshLaunchAuthorityScope, FreshOperationEntry, FreshOperationScope, FreshTaskOperationLaunch,
    SubsidiaryScopePolicy,
};
use golish_agent_kit::harness::StageKind;

const COMPANY: &str = "广州有创网络科技有限公司";
const LOOPBACK_TARGET: &str = "http://127.0.0.1:18080";

fn scoping_to_candidate() -> HashSet<StageKind> {
    HashSet::from([
        StageKind::Scoping,
        StageKind::TargetIntel,
        StageKind::ExternalAttackSurface,
        StageKind::Enumeration,
        StageKind::VulnTriage,
        StageKind::AttackCandidate,
    ])
}

fn gui_launch(scope: FreshOperationScope) -> FreshTaskOperationLaunch {
    FreshTaskOperationLaunch::new(
        COMPANY,
        "red_team",
        FreshOperationEntry::FullProfile,
        scope,
        SubsidiaryScopePolicy::default(),
        None,
    )
    .expect("valid GUI red_team launch")
}

fn cli_launch(scope: FreshOperationScope) -> FreshTaskOperationLaunch {
    FreshTaskOperationLaunch::new(
        COMPANY,
        "red_team",
        FreshOperationEntry::StageSlice {
            entry_stage: StageKind::Scoping,
            allowlist: scoping_to_candidate(),
        },
        scope,
        SubsidiaryScopePolicy::default(),
        None,
    )
    .expect("valid CLI red_team launch")
}

#[test]
fn explicit_cli_company_confirms_org_while_both_sides_remain_target_empty() {
    let organization_id = uuid::Uuid::from_u128(0x4985d1d1843a43fcabda35d03441e7f2);
    let policy = SubsidiaryScopePolicy::default();
    let gui = gui_launch(
        FreshOperationScope::unconfirmed_subject(COMPANY).expect("valid GUI company subject"),
    );
    let cli = cli_launch(
        FreshOperationScope::confirmed_organization_intake(COMPANY, organization_id, None, &policy)
            .expect("valid explicit CLI organization"),
    );

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
            label: COMPANY.to_string(),
        }
    );
    assert_eq!(
        cli_authority.scope,
        FreshLaunchAuthorityScope::ConfirmedOrganizationIntake {
            subject_label: COMPANY.to_string(),
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
fn loopback_target_gui_and_cli_have_identical_explicit_authority() {
    let policy = SubsidiaryScopePolicy::default();
    let gui = gui_launch(
        FreshOperationScope::confirmed_target_intake(
            Some(COMPANY.to_string()),
            vec![LOOPBACK_TARGET.to_string()],
            None,
            None,
            &policy,
        )
        .expect("valid GUI loopback target intake"),
    );
    let cli = cli_launch(
        FreshOperationScope::confirmed_target_intake(
            Some(COMPANY.to_string()),
            vec![LOOPBACK_TARGET.to_string()],
            None,
            None,
            &policy,
        )
        .expect("valid CLI loopback target intake"),
    );

    let gui_authority = gui
        .normalized_authority_projection()
        .expect("valid GUI authority");
    let cli_authority = cli
        .normalized_authority_projection()
        .expect("valid CLI authority");

    assert_eq!(gui_authority, cli_authority);
    assert_eq!(gui_authority.profile_id, "red_team");
    assert_eq!(gui_authority.start_stage, StageKind::Scoping);
    assert_eq!(
        gui_authority.scope,
        FreshLaunchAuthorityScope::ConfirmedTargetIntake {
            subject_label: Some(COMPANY.to_string()),
        }
    );
    assert_eq!(
        gui_authority.current_invocation_targets,
        vec![LOOPBACK_TARGET.to_string()]
    );
    assert_eq!(gui_authority.organization_id, None);
    assert_eq!(gui_authority.runtime_scope, None);
    assert!(!gui_authority.subsidiary_policy.include_subsidiaries);
    assert_eq!(
        gui_authority.subsidiary_policy.ownership_threshold_percent,
        51
    );
    assert!(matches!(gui.entry, FreshOperationEntry::FullProfile));
    assert!(matches!(
        cli.entry,
        FreshOperationEntry::StageSlice {
            entry_stage: StageKind::Scoping,
            allowlist,
        } if allowlist == scoping_to_candidate()
    ));
}

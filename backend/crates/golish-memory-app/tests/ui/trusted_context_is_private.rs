use golish_memory_app::TrustedAuthorizationContext;

fn main() {
    let _forged = TrustedAuthorizationContext {
        principal_id: panic!(),
        project_scope_id: panic!(),
        operation_id: panic!(),
        stage_execution_id: panic!(),
        stage_run_unit_id: panic!(),
        worker_run_id: panic!(),
        stage_kind: panic!(),
        wave: panic!(),
        scope_snapshot_id: panic!(),
        scope_snapshot_hash: panic!(),
        organization_id: panic!(),
        frozen_organization_ids: panic!(),
        classification_ceiling: panic!(),
        allowed_classes: panic!(),
        allow_external_embedding: panic!(),
        server_token_cap: panic!(),
        server_now: panic!(),
    };
}

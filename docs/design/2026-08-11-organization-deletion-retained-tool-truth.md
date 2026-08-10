# Organization deletion with retained Tool Truth

## Problem

Two-phase organization deletion freezes the organization/Target set, delivers source invalidations, cleans external artifacts, then physically deletes the live rows. Tool Truth later made stage waves and downstream Enumeration/Hypothesis authorities immutable, but several of those at-time rows still use live `organizations`/`targets` foreign keys. A target delete cascades into a bound `stage_asset_wave_items` row and is rejected by `tool_truth_bound_wave_source_immutable`; after that edge is detached, `audit_log.target_id ON DELETE SET NULL` tries to rewrite a bound evidence row and is rejected by `tool_truth_evidence_authority_immutable`.

The retained moresec.cn deletion job `aa504964-c34a-42d0-a488-6178e5b6c27d` demonstrates the mismatch: artifact cleanup succeeded, hard delete retried seven times, and one bound EAS wave plus its three members must remain immutable.

## Decision

Keep physical deletion of the live organization and Target rows. Preserve immutable evidence rows with their existing UUID values as at-time identities. Replace only the live-parent foreign keys on retained authority relations with trigger-enforced admission:

- inserts and changes of an organization reference must lock and match a live organization that is not in an active deletion job;
- inserts and changes of a Target reference must lock and match a live Target, including the exact organization/project tuple where the old compound FK required it, and must reject Targets in an active deletion job;
- parent deletion does not update or delete the retained child row, so canonical hashes and immutable membership remain unchanged;
- mutable/raw scan relations keep their existing `CASCADE`/`SET NULL` behavior;
- `SET NULL` live-target aliases on append-only/CAS-protected Candidate, Finding, Post-Exploit, Prepared Action and audit rows are also retained as at-time UUIDs, because nulling them would itself mutate sealed content. Their future writes use the same live-target admission trigger, and read paths still prove liveness by joining `targets`.

The retained relations are the Tool Truth wave header/member, referenced web-origin/API-endpoint/Enumeration observation spine, exact Enumeration groups/occurrences/lane receipts, coverage denominator members, Target Intel promotion references, and immutable Investigation/Hypothesis/report invalidation/revalidation organization authorities.

## Safety properties

1. No immutable Tool Truth row is updated or deleted by organization deletion.
2. No dangling identity can be introduced while the parent is live: the replacement trigger takes a key-share lock and checks exact identity.
3. Once deletion starts, new retained children for the frozen organization or Target are rejected.
4. Only parent deletion loses the live FK; all relationships inside the retained authority graph remain unchanged.
5. Existing two-phase deletion preconditions, invalidation delivery, artifact cleanup lease, retry backoff and retained job history remain the sole authorization for hard delete.
6. The two migrations are additive/forward-only and do not rewrite existing evidence; the second migration exists separately because the first had already been applied by the retained GUI database before the bound-audit edge was observed.

## Acceptance

- A fresh embedded-Postgres fixture with a bound stage wave completes organization hard delete, removes the live organization/Target, and retains the exact wave/member/binding rows.
- New references to the deleted organization/Target fail with FK-equivalent typed database errors.
- The retained moresec.cn job converges from `artifact_cleanup_succeeded` to `hard_delete_committed` after the migrated binary starts; the Target page refreshes without a second delete request.

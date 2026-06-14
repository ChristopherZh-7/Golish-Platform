-- Cascade target deletion when its owning organization (主体) is deleted.
--
-- Originally targets.organization_id was ON DELETE SET NULL (migration
-- 20260517194500_organizations_table.sql), so deleting an organization
-- DETACHED its targets — they survived with organization_id = NULL and the
-- Target panel dumped them into the virtual "未分组" (unassigned) bucket
-- instead of removing them. That contradicted the org repo's own "subtree
-- drops too" contract and surprised users (a deleted company left dozens of
-- orphan IPs floating in 未分组).
--
-- New behavior (user-confirmed 2026-06-14): deleting an organization deletes
-- the targets it owns. Combined with the org self-FK (parent_id ON DELETE
-- CASCADE), deleting a PARENT org cascades through every descendant org and
-- removes ALL of their targets in one shot; deleting a CHILD org removes just
-- that child's targets. Target-owned recon/scan/dns/security-analysis rows
-- already cascade off targets, so they drop too; findings/audit keep history
-- (their target_id is ON DELETE SET NULL).
--
-- Forward-only (invariant I10): swaps the FK action in place, no data change.
-- Idempotent — resolves and drops whatever FK currently guards
-- organization_id (the auto name is targets_organization_id_fkey) before
-- re-adding the CASCADE variant. All DDL is transactional and runs once.

DO $$
DECLARE
    fk_name text;
BEGIN
    SELECT con.conname INTO fk_name
    FROM pg_constraint con
    JOIN pg_class rel ON rel.oid = con.conrelid
    JOIN pg_attribute att
        ON att.attrelid = con.conrelid
       AND att.attnum = ANY (con.conkey)
    WHERE rel.relname = 'targets'
      AND att.attname = 'organization_id'
      AND con.contype = 'f';

    IF fk_name IS NOT NULL THEN
        EXECUTE format('ALTER TABLE targets DROP CONSTRAINT %I', fk_name);
    END IF;
END $$;

ALTER TABLE targets
    ADD CONSTRAINT targets_organization_id_fkey
    FOREIGN KEY (organization_id)
    REFERENCES organizations(id)
    ON DELETE CASCADE;

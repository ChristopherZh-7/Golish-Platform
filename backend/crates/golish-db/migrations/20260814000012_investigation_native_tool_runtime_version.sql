-- Native executables do not have a language-runtime version. Preserve the
-- exact empty value from ToolConfig instead of inventing a version string;
-- interpreted runtimes must continue to freeze a non-empty version.
ALTER TABLE investigation_dynamic_tool_inventory_members
    DROP CONSTRAINT investigation_dynamic_tool_inventory_memb_runtime_version_check;

ALTER TABLE investigation_dynamic_tool_inventory_members
    ADD CONSTRAINT investigation_dynamic_inventory_runtime_version_shape_check
    CHECK(runtime='native' OR BTRIM(runtime_version)<>'') NOT VALID;

ALTER TABLE investigation_dynamic_tool_inventory_members
    VALIDATE CONSTRAINT investigation_dynamic_inventory_runtime_version_shape_check;

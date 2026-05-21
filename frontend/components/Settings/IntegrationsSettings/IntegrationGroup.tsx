/**
 * Form for one credential group inside an [`IntegrationCard`].
 *
 * Renders one field per [`Field`] declared in the schema, plus the
 * Save / Clear / Test buttons wired to [`useIntegrationGroup`].
 *
 * Visual layout (vertical, label-on-top):
 *   <Field label>              <Field label>
 *   <Input>                    <Textarea spanning 4 rows>
 *   <Field label>
 *   <Select>
 *   ----------------- (divider) -----------------
 *   [Save] [Clear]    [Test]   <HealthPill>
 */

import { ExternalLink, Loader2, Save, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { IntegrationGroup as IntegrationGroupSchema } from "@/lib/api/integrations";
import { notify } from "@/lib/notify";
import { cn } from "@/lib/utils";
import { FieldRenderer } from "./fields/FieldRenderer";
import { useIntegrationGroup } from "./hooks/useIntegrationGroup";
import { TestButton } from "./TestButton";
import { safeI18nId, tWithDefault } from "./utils";

interface IntegrationGroupProps {
  toolId: string;
  group: IntegrationGroupSchema;
  /** Help URL inherited from the schema (used when the group has none of its own). */
  fallbackHelpUrl?: string;
}

export function IntegrationGroupForm({ toolId, group, fallbackHelpUrl }: IntegrationGroupProps) {
  const { t } = useTranslation();
  const helpUrl = group.help_url ?? fallbackHelpUrl;
  const safeId = safeI18nId(toolId);
  const groupDescription = group.description
    ? tWithDefault(
        t,
        `integrations.tool.${safeId}.group.${group.id}.description`,
        group.description
      )
    : undefined;
  const {
    status,
    snapshot,
    values,
    lastHealth,
    lastError,
    dirty,
    invalid,
    setField,
    save,
    clear,
    test,
  } = useIntegrationGroup({ toolId, groupId: group.id, fields: group.fields });

  const busy =
    status === "loading" || status === "saving" || status === "clearing" || status === "testing";

  const handleSave = async () => {
    await save();
    if (status !== "error") notify.success(t("integrations.saved"));
  };

  const handleClear = async () => {
    const ok = confirm(t("integrations.clearConfirm", { groupName: group.name }));
    if (!ok) return;
    await clear();
    if (status !== "error") notify.success(t("integrations.cleared"));
  };

  return (
    <div className="space-y-3">
      {(groupDescription || helpUrl) && (
        <div className="text-[11px] text-muted-foreground/70 leading-relaxed space-y-1">
          {groupDescription && <p>{groupDescription}</p>}
          {helpUrl && (
            <a
              href={helpUrl}
              target="_blank"
              rel="noreferrer"
              className="text-accent hover:underline inline-flex items-center gap-1"
            >
              <ExternalLink className="w-2.5 h-2.5" /> {t("integrations.help")}
            </a>
          )}
        </div>
      )}

      {status === "loading" && (
        <div className="flex items-center gap-2 text-[11px] text-muted-foreground/70 py-2">
          <Loader2 className="w-3 h-3 animate-spin" />
          {t("integrations.loading")}
        </div>
      )}

      {status === "error" && (
        <div className="rounded-md border border-red-500/30 bg-red-500/10 px-2.5 py-1.5 text-[11px] text-red-400">
          {t("integrations.loadFailed")}: {lastError}
        </div>
      )}

      {status !== "loading" && status !== "error" && (
        <div className="space-y-2.5">
          {group.fields.map((field) => {
            const inputId = `int-${toolId}-${group.id}-${field.key}`;
            const safeFieldKey = safeI18nId(field.key);
            const label = tWithDefault(
              t,
              `integrations.tool.${safeId}.group.${group.id}.field.${safeFieldKey}.label`,
              field.label
            );
            return (
              <div key={field.key} className="space-y-1">
                <label
                  htmlFor={inputId}
                  className="text-[11px] font-medium text-foreground/80 flex items-center gap-1"
                >
                  {label}
                  {field.required && <span className="text-red-400/80">*</span>}
                </label>
                <FieldRenderer
                  id={inputId}
                  field={field}
                  value={values[field.key] ?? ""}
                  onChange={(v) => setField(field.key, v)}
                  serverValue={snapshot[field.key] ?? null}
                  disabled={busy}
                />
              </div>
            );
          })}
        </div>
      )}

      {lastError && status !== "error" && status !== "loading" && (
        <div className="rounded-md border border-red-500/30 bg-red-500/10 px-2.5 py-1.5 text-[11px] text-red-400">
          {lastError}
        </div>
      )}

      <div className="flex items-center gap-2 pt-2 border-t border-border/15 flex-wrap">
        <button
          type="button"
          onClick={handleSave}
          disabled={busy || invalid || (!dirty && !hasUnsavedSecret(group, values))}
          className={cn(
            "px-2.5 py-1 text-[11px] rounded-md transition-colors inline-flex items-center gap-1",
            "bg-accent text-accent-foreground hover:bg-accent/90",
            "disabled:opacity-40 disabled:cursor-not-allowed"
          )}
        >
          <Save className="w-2.5 h-2.5" /> {t("integrations.save")}
        </button>
        <button
          type="button"
          onClick={handleClear}
          disabled={busy}
          className={cn(
            "px-2.5 py-1 text-[11px] rounded-md transition-colors inline-flex items-center gap-1",
            "bg-muted/50 text-foreground/70 hover:bg-muted",
            "disabled:opacity-40"
          )}
        >
          <Trash2 className="w-2.5 h-2.5" /> {t("integrations.clear")}
        </button>
        <div className="flex-1" />
        <TestButton
          onClick={test}
          busy={status === "testing"}
          health={lastHealth}
          hasTestRecipe={Boolean(group.test)}
          disabled={busy && status !== "testing"}
        />
      </div>
    </div>
  );
}

/** True when at least one secret field has new (non-empty) input the
 * user hasn't saved yet. "Save" stays enabled in that case even if
 * `dirty` was already reset (e.g. after a snapshot reload). */
function hasUnsavedSecret(group: IntegrationGroupSchema, values: Record<string, string>): boolean {
  return group.fields.some(
    (f) =>
      (f.type === "secret_text" || f.type === "secret_textarea") && (values[f.key] ?? "").length > 0
  );
}

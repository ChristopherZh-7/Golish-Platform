/**
 * Collapsible row for one [`ResolvedIntegration`].
 *
 * Visual style matches `VaultSettings` (small font sizes, muted
 * foreground with opacity, no emoji). Expanded body renders one
 * [`IntegrationGroupForm`] per declared group with an inline sub-nav
 * for multi-group integrations (ENScan_GO's 5 cookies).
 */

import { ChevronDown, ChevronRight, ExternalLink, KeyRound } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { ResolvedIntegration } from "@/lib/api/integrations";
import { cn } from "@/lib/utils";
import { IntegrationGroupForm } from "./IntegrationGroup";
import { safeI18nId, tWithDefault } from "./utils";

interface IntegrationCardProps {
  integration: ResolvedIntegration;
  /** When provided, the card mounts already expanded. Used by tests +
   *  by parent search-result auto-expansion. */
  defaultOpen?: boolean;
}

export function IntegrationCard({ integration, defaultOpen = false }: IntegrationCardProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(defaultOpen);
  const [activeGroup, setActiveGroup] = useState<string>(integration.schema.groups[0]?.id ?? "");

  const storageLabel = storageLabelFor(integration.schema.storage.type, t);
  const groups = integration.schema.groups;
  const activeGroupSchema = groups.find((g) => g.id === activeGroup) ?? groups[0];
  const safeId = safeI18nId(integration.tool_id);
  const displayName = tWithDefault(
    t,
    `integrations.tool.${safeId}.display_name`,
    integration.schema.display_name
  );
  const description = integration.schema.description
    ? tWithDefault(t, `integrations.tool.${safeId}.description`, integration.schema.description)
    : undefined;

  const helpUrl = integration.schema.help_url;

  return (
    <div
      className={cn(
        "rounded-lg border transition-colors",
        open
          ? "border-border/30 bg-[var(--bg-hover)]/15"
          : "border-border/15 hover:border-border/30 bg-[var(--bg-hover)]/10"
      )}
      data-tool-id={integration.tool_id}
    >
      <div className="w-full flex items-center gap-2 px-3 py-2 cursor-pointer text-left group">
        <button
          type="button"
          onClick={() => setOpen((o) => !o)}
          className="flex items-center gap-2 flex-1 min-w-0"
        >
          {open ? (
            <ChevronDown className="w-3 h-3 text-muted-foreground/40 flex-shrink-0" />
          ) : (
            <ChevronRight className="w-3 h-3 text-muted-foreground/40 flex-shrink-0" />
          )}
          <KeyRound className="w-3 h-3 text-accent/60 flex-shrink-0" />
          <span className="text-[12px] font-medium text-foreground/80 truncate">{displayName}</span>
        </button>
        {integration.schema.category && (
          <Pill>
            {t(`integrations.category.${integration.schema.category}`, {
              defaultValue: integration.schema.category,
            })}
          </Pill>
        )}
        <Pill>{storageLabel}</Pill>
        {groups.length > 1 && <Pill>{t("integrations.groupCount", { count: groups.length })}</Pill>}
        {helpUrl && (
          <a
            href={helpUrl}
            target="_blank"
            rel="noreferrer"
            onClick={(e) => e.stopPropagation()}
            title={t("integrations.signupLink")}
            className="flex items-center gap-0.5 text-[10px] px-1.5 py-0.5 rounded-full bg-accent/10 text-accent hover:bg-accent/20 transition-colors flex-shrink-0"
          >
            <ExternalLink className="w-2.5 h-2.5" />
            <span>{t("integrations.signup")}</span>
          </a>
        )}
        <span className="text-[9px] text-muted-foreground/30 font-mono flex-shrink-0 ml-1">
          {integration.tool_id}
        </span>
      </div>

      {open && (
        <div className="border-t border-border/15 px-3 py-3 space-y-3">
          {description && (
            <p className="text-[11px] text-muted-foreground/70 leading-relaxed">{description}</p>
          )}

          {groups.length > 1 && (
            <div className="flex items-center gap-1 flex-wrap border-b border-border/15 pb-2">
              {groups.map((g) => (
                <button
                  key={g.id}
                  type="button"
                  onClick={() => setActiveGroup(g.id)}
                  className={cn(
                    "px-2 py-0.5 text-[11px] rounded-md transition-colors",
                    activeGroup === g.id
                      ? "bg-accent/10 text-accent"
                      : "text-muted-foreground/60 hover:text-foreground hover:bg-[var(--bg-hover)]/40"
                  )}
                >
                  {tWithDefault(t, `integrations.tool.${safeId}.group.${g.id}.name`, g.name)}
                </button>
              ))}
            </div>
          )}

          {activeGroupSchema && (
            <IntegrationGroupForm
              key={activeGroupSchema.id}
              toolId={integration.tool_id}
              group={activeGroupSchema}
              fallbackHelpUrl={integration.schema.help_url}
            />
          )}
        </div>
      )}
    </div>
  );
}

function Pill({ children }: { children: React.ReactNode }) {
  return (
    <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-muted/20 text-muted-foreground/50 font-medium flex-shrink-0">
      {children}
    </span>
  );
}

function storageLabelFor(
  type: "vault" | "external_file" | "settings",
  t: (key: string, opts?: Record<string, unknown>) => string
): string {
  switch (type) {
    case "vault":
      return t("integrations.storage.vault");
    case "external_file":
      return t("integrations.storage.external_file");
    case "settings":
      return t("integrations.storage.settings");
  }
}

/**
 * "Test connection" button + IntegrationHealth pill.
 *
 * Status mapping (mirrors VaultSettings' pill style):
 *   healthy       → emerald (ShieldCheck)
 *   invalid       → red (ShieldX)
 *   expired       → amber (Clock)
 *   rate_limited  → amber (Hourglass)
 *   unknown       → muted (HelpCircle)
 */

import {
  Clock,
  HelpCircle,
  Hourglass,
  Loader2,
  PlayCircle,
  ShieldCheck,
  ShieldX,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import type { IntegrationHealth } from "@/lib/api/integrations";
import { cn } from "@/lib/utils";

interface TestButtonProps {
  onClick: () => void;
  busy: boolean;
  /** When `null`, no test has run yet. */
  health: IntegrationHealth | null;
  /** Hide the button when the schema declares no `test` recipe. */
  hasTestRecipe: boolean;
  disabled?: boolean;
}

export function TestButton({ onClick, busy, health, hasTestRecipe, disabled }: TestButtonProps) {
  const { t } = useTranslation();

  if (!hasTestRecipe) return null;

  return (
    <div className="flex items-center gap-2 flex-wrap">
      <button
        type="button"
        onClick={onClick}
        disabled={busy || disabled}
        className={cn(
          "text-[11px] px-2.5 py-1 rounded-md border transition-colors inline-flex items-center gap-1",
          "border-border/30 bg-[var(--bg-hover)]/30 text-foreground/80 hover:bg-[var(--bg-hover)]/60",
          "disabled:opacity-50 disabled:cursor-not-allowed"
        )}
      >
        {busy ? (
          <Loader2 className="w-2.5 h-2.5 animate-spin" />
        ) : (
          <PlayCircle className="w-2.5 h-2.5" />
        )}
        {busy ? t("integrations.testing") : t("integrations.testButton")}
      </button>
      {health && <HealthPill health={health} />}
    </div>
  );
}

function HealthPill({ health }: { health: IntegrationHealth }) {
  const { t } = useTranslation();
  const meta = (() => {
    switch (health.status) {
      case "healthy":
        return {
          label: t("integrations.health.healthy"),
          icon: <ShieldCheck className="w-2.5 h-2.5" />,
          cls: "bg-emerald-500/15 text-emerald-400",
        };
      case "invalid":
        return {
          label: t("integrations.health.invalid"),
          icon: <ShieldX className="w-2.5 h-2.5" />,
          cls: "bg-red-500/15 text-red-400",
        };
      case "expired":
        return {
          label: t("integrations.health.expired"),
          icon: <Clock className="w-2.5 h-2.5" />,
          cls: "bg-amber-500/15 text-amber-400",
        };
      case "rate_limited":
        return {
          label: t("integrations.health.rate_limited"),
          icon: <Hourglass className="w-2.5 h-2.5" />,
          cls: "bg-amber-500/15 text-amber-400",
        };
      default:
        return {
          label: t("integrations.health.unknown"),
          icon: <HelpCircle className="w-2.5 h-2.5" />,
          cls: "bg-muted/20 text-muted-foreground/60",
        };
    }
  })();
  return (
    <span
      className={cn(
        "text-[10px] font-medium px-1.5 py-0.5 rounded-full inline-flex items-center gap-1",
        meta.cls
      )}
      title={health.message}
    >
      {meta.icon}
      {meta.label}
    </span>
  );
}

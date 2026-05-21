/**
 * Conditional "Auto-capture" trigger for the IntegrationGroup toolbar.
 *
 * Renders the ⚡ button **only when `group.capture` is set** —
 * absence ⇒ returns `null` so the form stays identical to the
 * pre-capture experience for groups that haven't opted in (zero
 * regression for the 5 intel providers / GitHub Token / non-capture
 * ENScan groups).
 *
 * Style matches the existing Save / Clear / Test buttons in
 * `IntegrationGroup.tsx` (small text, accent border, lucide icon
 * left-of-label) — we deliberately avoid the heavier `<Button>` UI
 * primitive so the row's visual rhythm stays intact.
 */

import { Wand2 } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import type { IntegrationGroup } from "@/lib/api/integrations";
import { cn } from "@/lib/utils";

export interface CaptureButtonProps {
  toolId: string;
  group: IntegrationGroup;
  /**
   * Disabled while a session is in-flight (the parent toolbar passes
   * `session?.state` non-terminal as the truthy signal).
   */
  disabled?: boolean;
  /** Fires the parent hook's `start(toolId, groupId)`. */
  onStart: (toolId: string, groupId: string) => void;
}

export function CaptureButton({ toolId, group, disabled, onStart }: CaptureButtonProps) {
  const { t } = useTranslation();
  if (!group.capture) return null;

  return (
    <TooltipProvider delayDuration={300}>
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            type="button"
            disabled={disabled}
            onClick={() => onStart(toolId, group.id)}
            aria-label={t("integrations.capture.button.label")}
            className={cn(
              "px-2.5 py-1 text-[11px] rounded-md transition-colors inline-flex items-center gap-1",
              "border border-amber-400/40 bg-amber-500/10 text-amber-200 hover:bg-amber-500/20",
              "disabled:opacity-40 disabled:cursor-not-allowed"
            )}
          >
            <Wand2 className="w-2.5 h-2.5" />
            {t("integrations.capture.button.label")}
          </button>
        </TooltipTrigger>
        <TooltipContent side="top">{t("integrations.capture.button.tooltip")}</TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}

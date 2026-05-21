/**
 * Inline status banner shown inside an [`IntegrationGroupForm`] toolbar
 * while a capture session is active or has just terminated.
 *
 * Renders one of 8 states (`CaptureState`):
 *
 *   waiting_login / navigating / extracting → spinner + countdown
 *   captured                                 → green check + count
 *   partial                                  → yellow warning + breakdown
 *   timeout                                  → red clock + i18n message
 *   failed                                   → red X + engine error message
 *   cancelled                                → gray X + i18n message
 *
 * In-flight states also render an inline Cancel button next to the
 * label. Terminal states are read-only.
 */

import { AlertTriangle, CheckCircle2, Clock, Loader2, XCircle } from "lucide-react";
import type { ReactElement } from "react";
import { useTranslation } from "react-i18next";

import type { CaptureSessionInfo, CaptureState } from "@/lib/api/integrations";
import { cn } from "@/lib/utils";

const INFLIGHT_STATES: ReadonlySet<CaptureState> = new Set([
  "waiting_login",
  "navigating",
  "extracting",
]);

function iconFor(state: CaptureState): ReactElement {
  switch (state) {
    case "waiting_login":
    case "navigating":
    case "extracting":
      return <Loader2 className="w-3 h-3 animate-spin" />;
    case "captured":
      return <CheckCircle2 className="w-3 h-3 text-emerald-400" />;
    case "partial":
      return <AlertTriangle className="w-3 h-3 text-amber-400" />;
    case "failed":
      return <XCircle className="w-3 h-3 text-red-400" />;
    case "timeout":
      return <Clock className="w-3 h-3 text-red-400" />;
    case "cancelled":
      return <XCircle className="w-3 h-3 text-muted-foreground" />;
  }
}

function colorFor(state: CaptureState): string {
  switch (state) {
    case "waiting_login":
    case "navigating":
    case "extracting":
      return "border-amber-400/30 bg-amber-500/10 text-amber-200";
    case "captured":
      return "border-emerald-400/30 bg-emerald-500/10 text-emerald-200";
    case "partial":
      return "border-amber-400/40 bg-amber-500/15 text-amber-100";
    case "failed":
    case "timeout":
      return "border-red-500/30 bg-red-500/10 text-red-300";
    case "cancelled":
      return "border-border/30 bg-muted/30 text-muted-foreground";
  }
}

export interface CaptureStatusToastProps {
  session: CaptureSessionInfo | null;
  remainingSecs: number;
  /** Surface IPC errors from `proceedAfterConfirm` (CAPTURE_NO_RECIPE,
   *  CAPTURE_ALREADY_RUNNING, WEBVIEW_CREATE_FAILED, …). */
  startError?: string | null;
  onCancel: () => void;
}

export function CaptureStatusToast({
  session,
  remainingSecs,
  startError,
  onCancel,
}: CaptureStatusToastProps) {
  const { t } = useTranslation();

  if (!session && startError) {
    return (
      <div
        className={cn(
          "flex items-center gap-2 rounded-md border px-2.5 py-1.5 text-[11px]",
          "border-red-500/30 bg-red-500/10 text-red-300"
        )}
        role="status"
      >
        <XCircle className="w-3 h-3" />
        <span className="flex-1">{startError}</span>
      </div>
    );
  }

  if (!session) return null;

  const state = session.state;
  const isInflight = INFLIGHT_STATES.has(state);

  const text = (() => {
    switch (state) {
      case "waiting_login":
        return t("integrations.capture.toast.waitingLogin", {
          remaining: remainingSecs,
        });
      case "navigating":
        return t("integrations.capture.toast.navigating");
      case "extracting":
        return t("integrations.capture.toast.extracting");
      case "captured":
        return t("integrations.capture.toast.captured", {
          count: session.captured_fields?.length ?? 0,
        });
      case "partial":
        return t("integrations.capture.toast.partial", {
          captured: session.captured_fields?.length ?? 0,
          failed: session.failed_rules?.length ?? 0,
        });
      case "timeout":
        return t("integrations.capture.toast.timeout");
      case "failed":
        return session.error_message ?? t("integrations.capture.toast.failed");
      case "cancelled":
        return t("integrations.capture.toast.cancelled");
    }
  })();

  return (
    <div
      className={cn(
        "flex items-center gap-2 rounded-md border px-2.5 py-1.5 text-[11px]",
        colorFor(state)
      )}
      role="status"
      data-capture-state={state}
    >
      {iconFor(state)}
      <span className="flex-1">{text}</span>
      {isInflight && (
        <button
          type="button"
          onClick={onCancel}
          className={cn(
            "px-2 py-0.5 text-[10px] rounded transition-colors",
            "bg-muted/40 hover:bg-muted/60 text-foreground/80"
          )}
        >
          {t("integrations.capture.dialog.cancel")}
        </button>
      )}
    </div>
  );
}

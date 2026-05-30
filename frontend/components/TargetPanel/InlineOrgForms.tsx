/**
 * Inline create/edit forms for the org tree sidebar.
 *
 * Extracted verbatim from `TargetGroupedView.tsx`:
 *  - `InlineCreateOrgForm` (was `renderInlineCreateOrgForm`) — create an org,
 *    used both inside a tree node and at the root ("new top-level org").
 *  - `InlineAddTargetForm` (was `renderInlineAddTargetForm`) — add a target.
 *  - `InlineOrgEditForm` (was `renderOrgEditForm`) — rename / re-own an org.
 *
 * Form state stays owned by `TargetGroupedView`; these are controlled inputs.
 */

import { Building2, Check, Crosshair, X } from "lucide-react";
import type { Dispatch, SetStateAction } from "react";
import { cn } from "@/lib/utils";

interface InlineCreateOrgFormProps {
  parentId: string | null;
  depth: number;
  t: (key: string) => string;
  orgFormName: string;
  setOrgFormName: Dispatch<SetStateAction<string>>;
  orgFormOwner: string;
  setOrgFormOwner: Dispatch<SetStateAction<string>>;
  submitting: boolean;
  inlineError: string | null;
  handleCreateOrg: (parentId: string | null) => void;
  closeAllEditors: () => void;
}

export function InlineCreateOrgForm({
  parentId,
  depth,
  t,
  orgFormName,
  setOrgFormName,
  orgFormOwner,
  setOrgFormOwner,
  submitting,
  inlineError,
  handleCreateOrg,
  closeAllEditors,
}: InlineCreateOrgFormProps) {
  return (
    <div
      className="px-2 py-2 bg-muted/10 border-l-2 border-accent/40"
      style={{ marginLeft: `${8 + depth * 16}px` }}
    >
      <div className="flex items-center gap-2">
        <Building2 className="w-3 h-3 text-accent/70" />
        <input
          className="flex-1 text-xs bg-background border border-border/50 rounded px-2 py-1 outline-none focus:border-accent"
          placeholder={t("organizations.namePlaceholder")}
          value={orgFormName}
          onChange={(e) => setOrgFormName(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") handleCreateOrg(parentId);
            if (e.key === "Escape") closeAllEditors();
          }}
          // biome-ignore lint/a11y/noAutofocus: inline edit affordance needs immediate focus
          autoFocus
        />
        <input
          className="text-xs bg-background border border-border/50 rounded px-2 py-1 outline-none focus:border-accent w-32"
          placeholder={t("organizations.ownerPlaceholder")}
          value={orgFormOwner}
          onChange={(e) => setOrgFormOwner(e.target.value)}
        />
        <button
          type="button"
          className={cn(
            "p-1 rounded text-green-400 hover:bg-green-500/10",
            (!orgFormName.trim() || submitting) && "opacity-50"
          )}
          disabled={!orgFormName.trim() || submitting}
          onClick={() => handleCreateOrg(parentId)}
        >
          <Check className="w-3 h-3" />
        </button>
        <button
          type="button"
          className="p-1 rounded text-muted-foreground hover:text-foreground"
          onClick={closeAllEditors}
        >
          <X className="w-3 h-3" />
        </button>
      </div>
      {inlineError && <p className="text-[10px] text-red-400 mt-1">{inlineError}</p>}
    </div>
  );
}

interface InlineAddTargetFormProps {
  orgId: string;
  depth: number;
  t: (key: string) => string;
  targetFormValue: string;
  setTargetFormValue: Dispatch<SetStateAction<string>>;
  targetFormName: string;
  setTargetFormName: Dispatch<SetStateAction<string>>;
  submitting: boolean;
  inlineError: string | null;
  handleAddTargetSubmit: (orgId: string) => void;
  closeAllEditors: () => void;
}

export function InlineAddTargetForm({
  orgId,
  depth,
  t,
  targetFormValue,
  setTargetFormValue,
  targetFormName,
  setTargetFormName,
  submitting,
  inlineError,
  handleAddTargetSubmit,
  closeAllEditors,
}: InlineAddTargetFormProps) {
  return (
    <div
      className="px-2 py-2 bg-muted/10 border-l-2 border-accent/40"
      style={{ marginLeft: `${8 + depth * 16}px` }}
    >
      <div className="flex items-center gap-2">
        <Crosshair className="w-3 h-3 text-accent/70" />
        <input
          className="flex-1 text-xs bg-background border border-border/50 rounded px-2 py-1 outline-none focus:border-accent"
          placeholder={`${t("targets.value")} * (e.g. example.com)`}
          value={targetFormValue}
          onChange={(e) => setTargetFormValue(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") handleAddTargetSubmit(orgId);
            if (e.key === "Escape") closeAllEditors();
          }}
          // biome-ignore lint/a11y/noAutofocus: inline edit affordance needs immediate focus
          autoFocus
        />
        <input
          className="text-xs bg-background border border-border/50 rounded px-2 py-1 outline-none focus:border-accent w-32"
          placeholder={`${t("targets.name")} (${t("common.default")}: ${t("targets.value")})`}
          value={targetFormName}
          onChange={(e) => setTargetFormName(e.target.value)}
        />
        <button
          type="button"
          className={cn(
            "p-1 rounded text-green-400 hover:bg-green-500/10",
            (!targetFormValue.trim() || submitting) && "opacity-50"
          )}
          disabled={!targetFormValue.trim() || submitting}
          onClick={() => handleAddTargetSubmit(orgId)}
        >
          <Check className="w-3 h-3" />
        </button>
        <button
          type="button"
          className="p-1 rounded text-muted-foreground hover:text-foreground"
          onClick={closeAllEditors}
        >
          <X className="w-3 h-3" />
        </button>
      </div>
      {inlineError && <p className="text-[10px] text-red-400 mt-1">{inlineError}</p>}
    </div>
  );
}

interface InlineOrgEditFormProps {
  depth: number;
  t: (key: string) => string;
  orgFormName: string;
  setOrgFormName: Dispatch<SetStateAction<string>>;
  orgFormOwner: string;
  setOrgFormOwner: Dispatch<SetStateAction<string>>;
  submitting: boolean;
  handleSaveEditOrg: () => void;
  closeAllEditors: () => void;
}

export function InlineOrgEditForm({
  depth,
  t,
  orgFormName,
  setOrgFormName,
  orgFormOwner,
  setOrgFormOwner,
  submitting,
  handleSaveEditOrg,
  closeAllEditors,
}: InlineOrgEditFormProps) {
  return (
    <div
      className="flex items-center gap-2 px-2 py-1.5 bg-muted/15 rounded"
      style={{ paddingLeft: `${8 + depth * 16}px` }}
    >
      <Building2 className="w-3.5 h-3.5 text-accent/70" />
      <input
        className="flex-1 text-xs bg-background border border-border/50 rounded px-2 py-1 outline-none focus:border-accent"
        value={orgFormName}
        onChange={(e) => setOrgFormName(e.target.value)}
        // biome-ignore lint/a11y/noAutofocus: inline edit affordance needs immediate focus
        autoFocus
      />
      <input
        className="text-xs bg-background border border-border/50 rounded px-2 py-1 outline-none focus:border-accent w-32"
        placeholder={t("organizations.ownerPlaceholder")}
        value={orgFormOwner}
        onChange={(e) => setOrgFormOwner(e.target.value)}
      />
      <button
        type="button"
        className={cn(
          "p-1 rounded text-green-400 hover:bg-green-500/10",
          (!orgFormName.trim() || submitting) && "opacity-50"
        )}
        disabled={!orgFormName.trim() || submitting}
        onClick={handleSaveEditOrg}
      >
        <Check className="w-3 h-3" />
      </button>
      <button
        type="button"
        className="p-1 rounded text-muted-foreground hover:text-foreground"
        onClick={closeAllEditors}
      >
        <X className="w-3 h-3" />
      </button>
    </div>
  );
}

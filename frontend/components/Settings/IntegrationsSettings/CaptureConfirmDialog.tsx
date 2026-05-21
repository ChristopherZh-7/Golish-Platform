/**
 * Two-button confirmation dialog shown after the user clicks ⚡.
 *
 * Renders the recipe's `login_url`, the list of `target_field`s that
 * will be extracted, and the TTL — all interpolated into the
 * `integrations.capture.dialog.description` i18n key so translators
 * see one self-contained sentence.
 *
 * Uses the project's existing `Dialog` primitive (Radix) rather than
 * pulling in `@radix-ui/react-alert-dialog` — visually the difference
 * is negligible and the dependency surface stays smaller.
 */

import { useTranslation } from "react-i18next";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import type { CaptureRecipe } from "@/lib/api/integrations";
import { cn } from "@/lib/utils";

export interface CaptureConfirmDialogProps {
  open: boolean;
  onOpenChange: (v: boolean) => void;
  recipe: CaptureRecipe | null;
  onConfirm: () => void;
}

export function CaptureConfirmDialog({
  open,
  onOpenChange,
  recipe,
  onConfirm,
}: CaptureConfirmDialogProps) {
  const { t } = useTranslation();
  if (!recipe) return null;

  const fields = recipe.rules
    .map((r) => ("target_field" in r ? r.target_field : ""))
    .filter(Boolean)
    .join(", ");

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>{t("integrations.capture.dialog.title")}</DialogTitle>
          <DialogDescription>
            {t("integrations.capture.dialog.description", {
              url: recipe.login_url,
              fields,
              ttl: recipe.timeout_secs,
            })}
          </DialogDescription>
        </DialogHeader>
        {recipe.instructions && (
          <p className="text-[11px] text-muted-foreground/70 leading-relaxed">
            {recipe.instructions}
          </p>
        )}
        <DialogFooter>
          <button
            type="button"
            onClick={() => onOpenChange(false)}
            className={cn(
              "px-3 py-1.5 text-[12px] rounded-md transition-colors",
              "bg-muted/50 text-foreground/70 hover:bg-muted"
            )}
          >
            {t("integrations.capture.dialog.cancel")}
          </button>
          <button
            type="button"
            onClick={onConfirm}
            className={cn(
              "px-3 py-1.5 text-[12px] rounded-md transition-colors inline-flex items-center gap-1",
              "bg-accent text-accent-foreground hover:bg-accent/90"
            )}
          >
            {t("integrations.capture.dialog.start")}
          </button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

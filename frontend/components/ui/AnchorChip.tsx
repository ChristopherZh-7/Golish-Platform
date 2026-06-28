/**
 * AnchorChip
 *
 * Tool/sub-agent request ids remain part of the store and detail navigation,
 * but the visible anchor chip is intentionally hidden from the product UI.
 */
import { memo } from "react";

export type AnchorKind = "tool" | "agent";

interface AnchorChipProps {
  /** Optional precomputed debug anchor string. */
  anchor?: string | null;
  /** Automatic lookup inputs retained for call-site compatibility. */
  sessionId?: string | null;
  requestId?: string | null;
  /** Historical spacer prop retained for call-site compatibility. */
  reserveSpace?: boolean;
  className?: string;
}

export const AnchorChip = memo(function AnchorChip(_props: AnchorChipProps) {
  return null;
});

import { createPortal } from "react-dom";
import { Copy, Crosshair, Send, Zap } from "lucide-react";
import { copyToClipboard } from "@/lib/clipboard";
import { useTranslation } from "react-i18next";

export interface ZapContextMenuEntry {
  id: number;
  url: string;
}

interface ZapContextMenuProps {
  x: number;
  y: number;
  entry: ZapContextMenuEntry;
  onClose: () => void;
  onSendToRepeater: (entry: ZapContextMenuEntry) => void;
  onSendToIntruder?: (entry: ZapContextMenuEntry) => void;
  onActiveScan?: (url: string) => void;
}

const btnClass =
  "w-full flex items-center gap-2 px-3 py-1.5 text-left hover:bg-accent/10 transition-colors";

export function ZapContextMenu({
  x,
  y,
  entry,
  onClose,
  onSendToRepeater,
  onSendToIntruder,
  onActiveScan,
}: ZapContextMenuProps) {
  const { t } = useTranslation();

  return createPortal(
    <div
      className="fixed z-[9999] min-w-[160px] rounded-lg border border-border/20 bg-popover shadow-lg py-1 text-[11px]"
      style={{ top: y, left: x }}
    >
      <button
        type="button"
        className={btnClass}
        onClick={() => { onSendToRepeater(entry); onClose(); }}
      >
        <Send className="w-3 h-3 text-accent" />
        {t("security.sendToRepeater")}
      </button>
      {onSendToIntruder && (
        <button
          type="button"
          className={btnClass}
          onClick={() => { onSendToIntruder(entry); onClose(); }}
        >
          <Crosshair className="w-3 h-3 text-orange-400" />
          Send to Intruder
        </button>
      )}
      {onActiveScan && (
        <button
          type="button"
          className={btnClass}
          onClick={() => { onActiveScan(entry.url); onClose(); }}
        >
          <Zap className="w-3 h-3 text-orange-400" />
          {t("security.activeScan")}
        </button>
      )}
      <button
        type="button"
        className={btnClass}
        onClick={() => { copyToClipboard(entry.url); onClose(); }}
      >
        <Copy className="w-3 h-3 text-muted-foreground/50" />
        {t("security.copyUrl")}
      </button>
    </div>,
    document.body,
  );
}

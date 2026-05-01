import type React from "react";
import { FileText, FolderOpen } from "lucide-react";
import { useTranslation } from "react-i18next";

interface CreateInputProps {
  depth: number;
  creatingType: "file" | "folder" | null;
  newName: string;
  setNewName: (v: string) => void;
  newNameRef: React.RefObject<HTMLInputElement | null>;
  confirmCreate: () => void;
  cancelCreate: () => void;
}

export function CreateInput({
  depth, creatingType, newName, setNewName, newNameRef, confirmCreate, cancelCreate,
}: CreateInputProps) {
  const { t } = useTranslation();
  const pl = 8 + depth * 16;
  const icon = creatingType === "folder"
    ? <FolderOpen className="w-3.5 h-3.5 text-amber-400/70 flex-shrink-0" />
    : <FileText className="w-3.5 h-3.5 text-blue-400/60 flex-shrink-0" />;
  return (
    <div className="flex items-center gap-1.5 py-0.5 pr-2" style={{ paddingLeft: pl }}>
      {icon}
      <input
        ref={newNameRef}
        value={newName}
        onChange={(e) => setNewName(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") confirmCreate();
          if (e.key === "Escape") cancelCreate();
        }}
        onBlur={() => { if (!newName.trim()) cancelCreate(); else confirmCreate(); }}
        placeholder={creatingType === "folder" ? t("wiki.folderName") : t("wiki.fileName")}
        className="flex-1 px-1.5 py-0.5 text-[11px] rounded bg-background border border-accent/40 text-foreground placeholder:text-muted-foreground/30 outline-none"
      />
    </div>
  );
}

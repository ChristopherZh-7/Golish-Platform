import { homeDir } from "@tauri-apps/api/path";
import { open as openFolderDialog } from "@tauri-apps/plugin-dialog";
import { FolderOpen, X } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import type { ProjectFormData } from "@/lib/projects";

const DEFAULT_PROJECTS_DIR = "golish-platform";

interface SetupProjectModalProps {
  isOpen: boolean;
  onClose: () => void;
  onSubmit: (projectData: ProjectFormData) => void;
}

interface FormState {
  name: string;
  rootPath: string;
}

export function SetupProjectModal({ isOpen, onClose, onSubmit }: SetupProjectModalProps) {
  const { t } = useTranslation();
  const [formData, setFormData] = useState<FormState>({
    name: "",
    rootPath: "",
  });
  const [targetsText, setTargetsText] = useState("");

  useEffect(() => {
    if (isOpen && !formData.rootPath) {
      homeDir()
        .then((dir) => {
          const defaultPath = `${dir.replace(/\/$/, "")}/${DEFAULT_PROJECTS_DIR}`;
          setFormData((prev) => (prev.rootPath ? prev : { ...prev, rootPath: defaultPath }));
        })
        .catch(() => {});
    }
  }, [isOpen, formData.rootPath]);

  const handleChange = useCallback(<K extends keyof FormState>(field: K, value: FormState[K]) => {
    setFormData((prev) => ({
      ...prev,
      [field]: value,
    }));
  }, []);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!formData.name.trim() || !formData.rootPath.trim()) return;
    const parentDir = formData.rootPath.replace(/\/$/, "");
    const targets = targetsText
      .split(/[\n,]+/)
      .map((s) => s.trim())
      .filter(Boolean);
    onSubmit({
      name: formData.name.trim(),
      rootPath: `${parentDir}/${formData.name.trim()}`,
      targets: targets.length > 0 ? targets : undefined,
    });
    setTargetsText("");
    onClose();
  };

  const handlePickFolder = useCallback(async () => {
    const selected = await openFolderDialog({
      directory: true,
      multiple: false,
      title: "Select project root folder",
    });

    if (selected) {
      handleChange("rootPath", selected);
      if (!formData.name.trim()) {
        const folderName = selected.split("/").pop() || selected.split("\\").pop() || "";
        if (folderName) {
          handleChange("name", folderName);
        }
      }
    }
  }, [handleChange, formData.name]);

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      <div className="absolute inset-0 bg-black/70 backdrop-blur-sm" onClick={onClose} />

      <div className="relative bg-card border border-border rounded-lg shadow-2xl w-full max-w-lg overflow-hidden">
        {/* Header */}
        <div className="flex items-center justify-between p-4 border-b border-border">
          <h2 className="text-lg font-semibold text-foreground">{t("projectSetup.title")}</h2>
          <button
            type="button"
            onClick={onClose}
            className="p-1 hover:bg-muted rounded transition-colors"
          >
            <X size={20} className="text-muted-foreground" />
          </button>
        </div>

        {/* Form */}
        <form onSubmit={handleSubmit} className="p-6 space-y-5 max-h-[80vh] overflow-y-auto">
          {/* Parent directory */}
          <div>
            <span className="block text-xs text-muted-foreground mb-1.5">
              {t("projectSetup.parentDirectory")}
            </span>
            <div className="flex items-center space-x-2">
              <label className="flex-1">
                <input
                  type="text"
                  value={formData.rootPath}
                  onChange={(e) => handleChange("rootPath", e.target.value)}
                  placeholder="/path/to/project"
                  className="w-full bg-background border border-border rounded-md px-3 py-2 text-sm text-foreground/90 placeholder-muted-foreground/50 font-mono focus:outline-none focus:border-primary focus:ring-1 focus:ring-primary transition-colors"
                />
              </label>
              <button
                type="button"
                onClick={handlePickFolder}
                className="h-[38px] px-3 bg-secondary border border-border rounded-md hover:bg-muted transition-colors"
              >
                <FolderOpen size={16} className="text-muted-foreground" />
              </button>
            </div>
          </div>

          {/* Project Name */}
          <div>
            <label className="block">
              <span className="block text-xs text-muted-foreground mb-1.5">
                {t("projectSetup.projectName")}
              </span>
              <input
                type="text"
                value={formData.name}
                onChange={(e) => handleChange("name", e.target.value)}
                placeholder={t("projectSetup.projectNamePlaceholder")}
                className="w-full bg-background border border-border rounded-md px-3 py-2 text-sm text-foreground/90 placeholder-muted-foreground/50 focus:outline-none focus:border-primary focus:ring-1 focus:ring-primary transition-colors"
              />
            </label>
          </div>

          {/* Initial targets — always available under Schema E */}
          <div>
            <label className="block">
              <span className="block text-xs text-muted-foreground mb-1.5">
                {t("projectSetup.initialTargets")}{" "}
                <span className="text-muted-foreground/50">
                  {t("projectSetup.initialTargetsOptional")}
                </span>
              </span>
              <textarea
                value={targetsText}
                onChange={(e) => setTargetsText(e.target.value)}
                placeholder={"example.com\n192.168.1.0/24\nhttps://app.example.com"}
                rows={3}
                className="w-full bg-background border border-border rounded-md px-3 py-2 text-sm text-foreground/90 placeholder-muted-foreground/50 font-mono focus:outline-none focus:border-primary focus:ring-1 focus:ring-primary transition-colors resize-none"
              />
              <span className="block text-[11px] text-muted-foreground/60 mt-1">
                {t("projectSetup.initialTargetsHint")}
              </span>
            </label>
          </div>

          {formData.rootPath && formData.name.trim() && (
            <div className="text-xs text-muted-foreground font-mono bg-background rounded-md px-3 py-2 border border-secondary">
              {t("projectSetup.projectPath")}: {formData.rootPath.replace(/\/$/, "")}/
              {formData.name.trim()}
            </div>
          )}
        </form>

        {/* Footer */}
        <div className="flex items-center justify-end space-x-3 p-4 border-t border-border bg-card">
          <button
            type="button"
            onClick={onClose}
            className="px-4 py-2 text-sm font-medium text-foreground/80 bg-secondary border border-border rounded-md hover:bg-muted transition-colors"
          >
            {t("common.cancel")}
          </button>
          <button
            type="submit"
            onClick={handleSubmit}
            disabled={!formData.name.trim() || !formData.rootPath.trim()}
            className="px-4 py-2 text-sm font-medium text-primary-foreground bg-primary rounded-md hover:bg-primary/90 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {t("projectSetup.create")}
          </button>
        </div>
      </div>
    </div>
  );
}

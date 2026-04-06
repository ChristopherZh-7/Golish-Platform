import { homeDir } from "@tauri-apps/api/path";
import { open as openFolderDialog } from "@tauri-apps/plugin-dialog";
import { FolderOpen, X } from "lucide-react";
import { useCallback, useEffect, useState } from "react";

import type { ProjectFormData } from "@/lib/projects";

const DEFAULT_PROJECTS_DIR = "golish-platform";

interface SetupProjectModalProps {
  isOpen: boolean;
  onClose: () => void;
  onSubmit: (projectData: ProjectFormData) => void;
}

export function SetupProjectModal({ isOpen, onClose, onSubmit }: SetupProjectModalProps) {
  const [formData, setFormData] = useState<ProjectFormData>({
    name: "",
    rootPath: "",
  });

  useEffect(() => {
    if (isOpen && !formData.rootPath) {
      homeDir().then((dir) => {
        const defaultPath = `${dir.replace(/\/$/, "")}/${DEFAULT_PROJECTS_DIR}`;
        setFormData((prev) => prev.rootPath ? prev : { ...prev, rootPath: defaultPath });
      }).catch(() => {});
    }
  }, [isOpen, formData.rootPath]);

  const handleChange = useCallback((field: keyof ProjectFormData, value: string) => {
    setFormData((prev) => ({
      ...prev,
      [field]: value,
    }));
  }, []);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!formData.name.trim() || !formData.rootPath.trim()) return;
    const parentDir = formData.rootPath.replace(/\/$/, "");
    onSubmit({ name: formData.name.trim(), rootPath: `${parentDir}/${formData.name.trim()}` });
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
      // Auto-fill name from folder name if empty
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
      {/* biome-ignore lint/a11y/useKeyWithClickEvents: backdrop dismiss */}
      {/* biome-ignore lint/a11y/noStaticElementInteractions: backdrop dismiss */}
      <div className="absolute inset-0 bg-black/70 backdrop-blur-sm" onClick={onClose} />

      <div className="relative bg-[#161b22] border border-[#30363d] rounded-lg shadow-2xl w-full max-w-md overflow-hidden">
        {/* Header */}
        <div className="flex items-center justify-between p-4 border-b border-[#30363d]">
          <h2 className="text-lg font-semibold text-white">New Project</h2>
          <button
            type="button"
            onClick={onClose}
            className="p-1 hover:bg-[#30363d] rounded transition-colors"
          >
            <X size={20} className="text-gray-400" />
          </button>
        </div>

        {/* Form */}
        <form onSubmit={handleSubmit} className="p-6 space-y-5">
          {/* Parent directory - project will be created as a subdirectory */}
          <div>
            <span className="block text-xs text-gray-400 mb-1.5">Parent Directory</span>
            <div className="flex items-center space-x-2">
              <label className="flex-1">
                <input
                  type="text"
                  value={formData.rootPath}
                  onChange={(e) => handleChange("rootPath", e.target.value)}
                  placeholder="/path/to/project"
                  className="w-full bg-[#0d1117] border border-[#30363d] rounded-md px-3 py-2 text-sm text-gray-200 placeholder-gray-600 font-mono focus:outline-none focus:border-[#58a6ff] focus:ring-1 focus:ring-[#58a6ff] transition-colors"
                />
              </label>
              <button
                type="button"
                onClick={handlePickFolder}
                className="h-[38px] px-3 bg-[#21262d] border border-[#30363d] rounded-md hover:bg-[#30363d] transition-colors"
              >
                <FolderOpen size={16} className="text-gray-400" />
              </button>
            </div>
          </div>

          {/* Project Name */}
          <div>
            <label className="block">
              <span className="block text-xs text-gray-400 mb-1.5">Project Name</span>
              <input
                type="text"
                value={formData.name}
                onChange={(e) => handleChange("name", e.target.value)}
                placeholder="my-project"
                className="w-full bg-[#0d1117] border border-[#30363d] rounded-md px-3 py-2 text-sm text-gray-200 placeholder-gray-600 focus:outline-none focus:border-[#58a6ff] focus:ring-1 focus:ring-[#58a6ff] transition-colors"
              />
            </label>
          </div>

          {formData.rootPath && formData.name.trim() && (
            <div className="text-xs text-gray-500 font-mono bg-[#0d1117] rounded-md px-3 py-2 border border-[#21262d]">
              Project path: {formData.rootPath.replace(/\/$/, "")}/{formData.name.trim()}
            </div>
          )}
        </form>

        {/* Footer */}
        <div className="flex items-center justify-end space-x-3 p-4 border-t border-[#30363d] bg-[#161b22]">
          <button
            type="button"
            onClick={onClose}
            className="px-4 py-2 text-sm font-medium text-gray-300 bg-[#21262d] border border-[#30363d] rounded-md hover:bg-[#30363d] transition-colors"
          >
            Cancel
          </button>
          <button
            type="submit"
            onClick={handleSubmit}
            disabled={!formData.name.trim() || !formData.rootPath.trim()}
            className="px-4 py-2 text-sm font-medium text-white bg-[#238636] rounded-md hover:bg-[#2ea043] transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            Create Project
          </button>
        </div>
      </div>
    </div>
  );
}

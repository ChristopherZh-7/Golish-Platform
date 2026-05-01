import { useEffect, useRef, useState } from "react";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { FolderOpen, Plus } from "lucide-react";
import { Markdown } from "@/components/Markdown/Markdown";
import { Button } from "@/components/ui/button";

export function MarkdownPreview({ content }: { content: string }) {
  return (
    <div className="p-4 overflow-auto h-full">
      <Markdown content={content} />
    </div>
  );
}

export function EditablePathBar({
  value,
  onNavigate,
}: {
  value: string;
  onNavigate: (path: string) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(value);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!editing) {
      setDraft(value);
    }
  }, [value, editing]);

  const startEditing = () => {
    setDraft(value);
    setEditing(true);
    requestAnimationFrame(() => {
      inputRef.current?.focus();
      inputRef.current?.select();
    });
  };

  const commit = () => {
    setEditing(false);
    const trimmed = draft.trim();
    if (trimmed && trimmed !== value) {
      onNavigate(trimmed);
    } else {
      setDraft(value);
    }
  };

  const cancel = () => {
    setEditing(false);
    setDraft(value);
  };

  if (!editing) {
    return (
      <button
        type="button"
        onClick={startEditing}
        className="font-mono text-[11px] truncate text-left hover:text-foreground transition-colors cursor-text min-w-0"
        title={`${value}\nClick to edit path`}
      >
        {value || "Browser"}
      </button>
    );
  }

  return (
    <input
      ref={inputRef}
      type="text"
      value={draft}
      onChange={(e) => setDraft(e.target.value)}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          commit();
        } else if (e.key === "Escape") {
          e.preventDefault();
          cancel();
        }
      }}
      onBlur={commit}
      className="font-mono text-[11px] bg-transparent border-b border-primary/50 outline-none text-foreground min-w-0 w-full"
    />
  );
}

export function FileOpenPrompt({
  workingDirectory,
  onOpen,
  onOpenBrowser,
  recentFiles,
}: {
  workingDirectory?: string | null;
  onOpen: (path: string) => void;
  onOpenBrowser: () => void;
  recentFiles: string[];
}) {
  const handleBrowse = async () => {
    const selected = await openFileDialog({
      directory: false,
      multiple: false,
      defaultPath: workingDirectory ?? undefined,
    });
    if (selected) {
      onOpen(selected);
    }
  };

  return (
    <div className="h-full flex flex-col items-center justify-center gap-6 px-6 text-center">
      <div className="space-y-2 max-w-xl">
        <p className="text-xs text-muted-foreground">Open a file to start editing</p>
        <p className="text-xs text-muted-foreground/80">
          Browse for a file or use the file browser.
        </p>
      </div>
      <div className="w-full max-w-xl flex flex-col items-stretch gap-3">
        <div className="flex gap-2">
          <Button onClick={handleBrowse} variant="default" className="flex-1 justify-center gap-2">
            <Plus className="h-4 w-4" />
            Open File
          </Button>
          <Button onClick={onOpenBrowser} variant="outline" className="flex-1 justify-center gap-2">
            <FolderOpen className="h-4 w-4" />
            Browse Files
          </Button>
        </div>
        {recentFiles.length > 0 && (
          <div className="w-full text-left mt-4">
            <p className="text-xs text-muted-foreground mb-2">Recent files:</p>
            <div className="grid gap-2">
              {recentFiles.slice(0, 5).map((file) => {
                const fileName = file.split("/").pop() || file;
                const parentDir = file.split("/").slice(-2, -1)[0];
                const displayPath = parentDir ? `${parentDir}/${fileName}` : fileName;
                return (
                  <button
                    key={file}
                    type="button"
                    onClick={() => onOpen(file)}
                    className="text-left text-xs font-mono px-3 py-2 rounded-md border border-border hover:border-primary/60 hover:bg-muted transition truncate"
                    title={file}
                  >
                    {displayPath}
                  </button>
                );
              })}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { logAudit } from "@/lib/audit";
import { MessageSquare, Plus, Trash2, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { getProjectPath } from "@/lib/projects";

interface Note {
  id: string;
  entity_type: string;
  entity_id: string;
  content: string;
  color: string;
  created_at: number;
  updated_at: number;
}

const COLORS = [
  { id: "yellow", bg: "bg-yellow-500/10 border-yellow-500/20", dot: "bg-yellow-400" },
  { id: "blue", bg: "bg-blue-500/10 border-blue-500/20", dot: "bg-blue-400" },
  { id: "green", bg: "bg-green-500/10 border-green-500/20", dot: "bg-green-400" },
  { id: "red", bg: "bg-red-500/10 border-red-500/20", dot: "bg-red-400" },
  { id: "purple", bg: "bg-purple-500/10 border-purple-500/20", dot: "bg-purple-400" },
];

export function QuickNotes({
  entityType,
  entityId,
  compact = false,
}: {
  entityType: string;
  entityId: string;
  compact?: boolean;
}) {
  const [notes, setNotes] = useState<Note[]>([]);
  const [showAdd, setShowAdd] = useState(false);
  const [newContent, setNewContent] = useState("");
  const [newColor, setNewColor] = useState("yellow");
  const [editingId, setEditingId] = useState<string | null>(null);
  const textRef = useRef<HTMLTextAreaElement>(null);

  const load = useCallback(async () => {
    try {
      const list = await invoke<Note[]>("notes_list", {
        entityType,
        entityId,
        projectPath: getProjectPath(),
      });
      setNotes(Array.isArray(list) ? list : []);
    } catch {
      setNotes([]);
    }
  }, [entityType, entityId]);

  useEffect(() => { load(); }, [load]);

  const handleAdd = useCallback(async () => {
    if (!newContent.trim()) return;
    try {
      await invoke("notes_add", {
        entityType,
        entityId,
        content: newContent.trim(),
        color: newColor,
        projectPath: getProjectPath(),
      });
      setNewContent("");
      setShowAdd(false);
      load();

      logAudit({ action: "note_added", category: "notes", details: `Added note to ${entityType}:${entityId}`, entityType, entityId });
    } catch { /* ignore */ }
  }, [newContent, newColor, entityType, entityId, load]);

  const handleUpdate = useCallback(async (id: string, content: string) => {
    try {
      await invoke("notes_update", { id, content, projectPath: getProjectPath() });
      setEditingId(null);
      load();
    } catch { /* ignore */ }
  }, [load]);

  const handleDelete = useCallback(async (id: string) => {
    try {
      await invoke("notes_delete", { id, projectPath: getProjectPath() });
      load();
    } catch { /* ignore */ }
  }, [load]);

  if (compact && notes.length === 0 && !showAdd) {
    return (
      <button
        onClick={() => { setShowAdd(true); requestAnimationFrame(() => textRef.current?.focus()); }}
        className="flex items-center gap-1 text-[9px] text-muted-foreground/30 hover:text-muted-foreground/60 transition-colors"
      >
        <MessageSquare className="w-2.5 h-2.5" />
        Add note
      </button>
    );
  }

  return (
    <div className="space-y-1.5">
      {notes.length > 0 && (
        <div className="space-y-1">
          {notes.map((note) => {
            const colorCfg = COLORS.find((c) => c.id === note.color) || COLORS[0];
            return (
              <div key={note.id} className={cn("rounded-md border px-2 py-1.5 group", colorCfg.bg)}>
                {editingId === note.id ? (
                  <textarea
                    defaultValue={note.content}
                    className="w-full text-[10px] bg-transparent outline-none resize-none"
                    rows={2}
                    autoFocus
                    onBlur={(e) => handleUpdate(note.id, e.target.value)}
                    onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); handleUpdate(note.id, e.currentTarget.value); } }}
                  />
                ) : (
                  <div className="flex items-start gap-1.5">
                    <div
                      className="flex-1 text-[10px] text-foreground/80 whitespace-pre-wrap cursor-pointer"
                      onClick={() => setEditingId(note.id)}
                    >
                      {note.content}
                    </div>
                    <button
                      onClick={() => handleDelete(note.id)}
                      className="p-0.5 opacity-0 group-hover:opacity-100 text-muted-foreground/30 hover:text-red-400 transition-all flex-shrink-0"
                    >
                      <Trash2 className="w-2.5 h-2.5" />
                    </button>
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}

      {showAdd ? (
        <div className="space-y-1.5">
          <textarea
            ref={textRef}
            value={newContent}
            onChange={(e) => setNewContent(e.target.value)}
            placeholder="Add a note..."
            className="w-full text-[10px] px-2 py-1.5 bg-background border border-border/30 rounded-md outline-none resize-none focus:border-accent/40"
            rows={2}
            onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); handleAdd(); } if (e.key === "Escape") setShowAdd(false); }}
          />
          <div className="flex items-center gap-1.5">
            <div className="flex items-center gap-1">
              {COLORS.map((c) => (
                <button
                  key={c.id}
                  onClick={() => setNewColor(c.id)}
                  className={cn(
                    "w-3 h-3 rounded-full transition-all",
                    c.dot,
                    newColor === c.id ? "ring-1 ring-offset-1 ring-foreground/30 ring-offset-background scale-110" : "opacity-50 hover:opacity-80",
                  )}
                />
              ))}
            </div>
            <div className="flex-1" />
            <button onClick={() => setShowAdd(false)} className="text-[9px] text-muted-foreground/40 hover:text-foreground transition-colors">
              <X className="w-3 h-3" />
            </button>
            <button onClick={handleAdd} disabled={!newContent.trim()}
              className="text-[9px] text-accent hover:text-accent/80 font-medium disabled:opacity-30 transition-colors">
              Save
            </button>
          </div>
        </div>
      ) : (
        <button
          onClick={() => { setShowAdd(true); requestAnimationFrame(() => textRef.current?.focus()); }}
          className="flex items-center gap-1 text-[9px] text-muted-foreground/30 hover:text-muted-foreground/60 transition-colors"
        >
          <Plus className="w-2.5 h-2.5" />
          Add note
        </button>
      )}
    </div>
  );
}

import { useCallback, useEffect, useMemo, useState } from "react";
import {
  BookOpen,
  ChevronDown,
  ChevronRight,
  Edit3,
  FolderOpen,
  Globe,
  Loader2,
  Plus,
  Save,
  Trash2,
  X,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Badge } from "@/components/ui/badge";
import {
  type SkillInfo,
  deleteSkill,
  listSkills,
  readSkillBody,
  saveSkill,
} from "@/lib/ai";
import { notify } from "@/lib/notify";

interface EditingSkill {
  name: string;
  description: string;
  body: string;
  scope: "global" | "project";
  isNew: boolean;
}

function emptySkill(scope: "global" | "project" = "global"): EditingSkill {
  return {
    name: "",
    description: "",
    body: "",
    scope,
    isNew: true,
  };
}

export function SkillsSettings() {
  const [skills, setSkills] = useState<SkillInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [expandedIds, setExpandedIds] = useState<Set<string>>(new Set());
  const [editingSkill, setEditingSkill] = useState<EditingSkill | null>(null);
  const [saving, setSaving] = useState(false);
  const [loadedBodies, setLoadedBodies] = useState<Record<string, string>>({});

  const loadSkills = useCallback(async () => {
    try {
      setLoading(true);
      const list = await listSkills();
      setSkills(list);
    } catch (err) {
      console.error("Failed to load skills:", err);
      notify.error("Failed to load skills");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadSkills();
  }, [loadSkills]);

  const toggleExpand = async (skill: SkillInfo) => {
    const key = skill.path;
    setExpandedIds((prev) => {
      const next = new Set(prev);
      if (next.has(key)) {
        next.delete(key);
      } else {
        next.add(key);
      }
      return next;
    });

    if (!loadedBodies[key]) {
      try {
        const body = await readSkillBody(skill.path);
        setLoadedBodies((prev) => ({ ...prev, [key]: body }));
      } catch (err) {
        console.error("Failed to load skill body:", err);
      }
    }
  };

  const startEditing = async (skill: SkillInfo) => {
    try {
      const body = loadedBodies[skill.path] || (await readSkillBody(skill.path));
      setEditingSkill({
        name: skill.name,
        description: skill.description,
        body,
        scope: skill.source === "project" ? "project" : "global",
        isNew: false,
      });
    } catch (err) {
      console.error("Failed to load skill for editing:", err);
      notify.error("Failed to load skill body");
    }
  };

  const startCreating = (scope: "global" | "project" = "global") => {
    setEditingSkill(emptySkill(scope));
  };

  const globalSkills = useMemo(() => skills.filter((s) => s.source !== "project"), [skills]);
  const projectSkills = useMemo(() => skills.filter((s) => s.source === "project"), [skills]);

  const cancelEditing = () => {
    setEditingSkill(null);
  };

  const handleSave = async () => {
    if (!editingSkill) return;
    if (!editingSkill.name.trim()) {
      notify.error("Skill name is required");
      return;
    }
    if (!/^[a-z0-9-]+$/.test(editingSkill.name)) {
      notify.error("Skill name must be lowercase alphanumeric with hyphens only");
      return;
    }

    try {
      setSaving(true);
      await saveSkill({
        name: editingSkill.name.trim(),
        description: editingSkill.description.trim(),
        body: editingSkill.body,
        scope: editingSkill.scope,
      });
      notify.success(`Skill "${editingSkill.name}" saved`);
      setEditingSkill(null);
      setLoadedBodies({});
      await loadSkills();
    } catch (err) {
      console.error("Failed to save skill:", err);
      notify.error(`Failed to save skill: ${err}`);
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async (skill: SkillInfo) => {
    try {
      await deleteSkill(skill.path);
      notify.success(`Skill "${skill.name}" deleted`);
      await loadSkills();
    } catch (err) {
      console.error("Failed to delete skill:", err);
      notify.error(`Failed to delete: ${err}`);
    }
  };

  // ── Editing form ──

  if (editingSkill) {
    return (
      <div className="space-y-6">
        <div className="flex items-center justify-between">
          <h4 className="text-sm font-medium text-accent">
            {editingSkill.isNew ? "Create New Skill" : `Edit: ${editingSkill.name}`}
          </h4>
          <div className="flex gap-2">
            <Button variant="ghost" size="sm" onClick={cancelEditing}>
              <X className="w-4 h-4 mr-1" /> Cancel
            </Button>
            <Button
              size="sm"
              onClick={handleSave}
              disabled={saving}
              className="bg-accent text-accent-foreground hover:bg-accent/90"
            >
              {saving ? (
                <Loader2 className="w-4 h-4 mr-1 animate-spin" />
              ) : (
                <Save className="w-4 h-4 mr-1" />
              )}
              Save
            </Button>
          </div>
        </div>

        <div className="grid grid-cols-2 gap-3">
          <div className="space-y-1">
            <label className="text-xs text-muted-foreground">
              Name {!editingSkill.isNew && <span className="text-[10px]">(read-only)</span>}
            </label>
            <Input
              value={editingSkill.name}
              onChange={(e) =>
                editingSkill.isNew &&
                setEditingSkill({
                  ...editingSkill,
                  name: e.target.value.replace(/[^a-z0-9-]/g, ""),
                })
              }
              readOnly={!editingSkill.isNew}
              placeholder="my-skill"
              className="bg-background border-border text-foreground h-9 font-mono text-xs"
            />
            <p className="text-[10px] text-muted-foreground">
              Lowercase, hyphens, numbers only
            </p>
          </div>
          <div className="space-y-1">
            <label className="text-xs text-muted-foreground">Scope</label>
            <div className="flex gap-1 h-9 items-center">
              <Button
                variant={editingSkill.scope === "global" ? "default" : "outline"}
                size="sm"
                onClick={() => setEditingSkill({ ...editingSkill, scope: "global" })}
                className={`h-8 px-3 text-xs gap-1 ${editingSkill.scope === "global" ? "bg-accent text-accent-foreground" : ""}`}
              >
                <Globe className="w-3 h-3" /> Global
              </Button>
              <Button
                variant={editingSkill.scope === "project" ? "default" : "outline"}
                size="sm"
                onClick={() => setEditingSkill({ ...editingSkill, scope: "project" })}
                className={`h-8 px-3 text-xs gap-1 ${editingSkill.scope === "project" ? "bg-accent text-accent-foreground" : ""}`}
              >
                <FolderOpen className="w-3 h-3" /> Project
              </Button>
            </div>
          </div>
        </div>

        <div className="space-y-1">
          <label className="text-xs text-muted-foreground">Description</label>
          <Input
            value={editingSkill.description}
            onChange={(e) =>
              setEditingSkill({ ...editingSkill, description: e.target.value })
            }
            placeholder="What this skill teaches the agent..."
            className="bg-background border-border text-foreground h-9"
          />
        </div>

        <div className="space-y-1">
          <label className="text-xs text-muted-foreground">Instructions (Markdown)</label>
          <Textarea
            value={editingSkill.body}
            onChange={(e) =>
              setEditingSkill({ ...editingSkill, body: e.target.value })
            }
            placeholder="You are an expert in..."
            className="bg-background border-border text-foreground font-mono text-xs min-h-[300px] resize-y"
          />
          <p className="text-[10px] text-muted-foreground">
            {editingSkill.body.length} characters - This content is injected into the agent
            prompt when the skill is matched.
          </p>
        </div>
      </div>
    );
  }

  // ── Skills list ──

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <Loader2 className="w-5 h-5 text-muted-foreground animate-spin" />
      </div>
    );
  }

  const renderSkillCard = (skill: SkillInfo) => {
    const isExpanded = expandedIds.has(skill.path);
    const body = loadedBodies[skill.path];

    return (
      <div
        key={skill.path}
        className="rounded-lg bg-muted border border-[var(--border-medium)] overflow-hidden"
      >
        <button
          type="button"
          onClick={() => toggleExpand(skill)}
          className="w-full flex items-center gap-3 px-4 py-3 text-left hover:bg-[var(--bg-hover)] transition-colors"
        >
          {isExpanded ? (
            <ChevronDown className="w-4 h-4 text-muted-foreground flex-shrink-0" />
          ) : (
            <ChevronRight className="w-4 h-4 text-muted-foreground flex-shrink-0" />
          )}
          <BookOpen className="w-4 h-4 text-accent flex-shrink-0" />
          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-2">
              <span className="text-sm font-medium text-foreground font-mono">{skill.name}</span>
              {skill.has_scripts && (
                <Badge variant="outline" className="text-[9px] px-1.5 py-0 h-4">scripts</Badge>
              )}
            </div>
            <p className="text-xs text-muted-foreground truncate">{skill.description}</p>
          </div>
        </button>

        {isExpanded && (
          <div className="px-4 pb-4 pt-1 border-t border-[var(--border-medium)] space-y-3">
            {body ? (
              <pre className="text-xs text-muted-foreground bg-background rounded p-3 max-h-[200px] overflow-auto whitespace-pre-wrap font-mono">
                {body}
              </pre>
            ) : (
              <div className="flex items-center gap-2 py-2">
                <Loader2 className="w-3 h-3 animate-spin text-muted-foreground" />
                <span className="text-xs text-muted-foreground">Loading...</span>
              </div>
            )}

            {skill.allowed_tools && skill.allowed_tools.length > 0 && (
              <div className="flex flex-wrap gap-1">
                {skill.allowed_tools.map((tool) => (
                  <Badge key={tool} variant="secondary" className="text-[10px] font-mono px-1.5 py-0">{tool}</Badge>
                ))}
              </div>
            )}

            <p className="text-[10px] text-muted-foreground/50 font-mono truncate flex items-center gap-1">
              <FolderOpen className="w-3 h-3" /> {skill.path}
            </p>

            <div className="flex gap-2 pt-1">
              <Button variant="outline" size="sm" onClick={() => startEditing(skill)} className="h-7 text-xs">
                <Edit3 className="w-3 h-3 mr-1" /> Edit
              </Button>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => handleDelete(skill)}
                className="h-7 text-xs text-muted-foreground hover:text-destructive"
              >
                <Trash2 className="w-3 h-3 mr-1" /> Delete
              </Button>
            </div>
          </div>
        )}
      </div>
    );
  };

  return (
    <div className="space-y-6">
      {/* Global Skills */}
      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Globe className="w-4 h-4 text-accent" />
            <h4 className="text-sm font-medium text-accent">Global Skills</h4>
            <span className="text-[10px] text-muted-foreground">~/.golish/skills/</span>
          </div>
          <Button
            size="sm"
            onClick={() => startCreating("global")}
            className="bg-accent text-accent-foreground hover:bg-accent/90 h-7"
          >
            <Plus className="w-3.5 h-3.5 mr-1" /> New
          </Button>
        </div>
        <div className="space-y-2">
          {globalSkills.map(renderSkillCard)}
          {globalSkills.length === 0 && (
            <p className="text-xs text-muted-foreground italic py-3 text-center">
              No global skills. Click &quot;New&quot; to create one.
            </p>
          )}
        </div>
      </div>

      <div className="border-t border-[var(--border-medium)]" />

      {/* Project Skills */}
      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <FolderOpen className="w-4 h-4 text-accent" />
            <h4 className="text-sm font-medium text-accent">Project Skills</h4>
            <span className="text-[10px] text-muted-foreground">.golish/skills/</span>
          </div>
          <Button
            size="sm"
            onClick={() => startCreating("project")}
            className="bg-accent text-accent-foreground hover:bg-accent/90 h-7"
          >
            <Plus className="w-3.5 h-3.5 mr-1" /> New
          </Button>
        </div>
        <div className="space-y-2">
          {projectSkills.map(renderSkillCard)}
          {projectSkills.length === 0 && (
            <p className="text-xs text-muted-foreground italic py-3 text-center">
              No project-specific skills. These are stored in your project&apos;s <code>.golish/skills/</code> directory.
            </p>
          )}
        </div>
      </div>

      <div className="text-xs text-muted-foreground border-t border-[var(--border-medium)] pt-4">
        <p>
          <strong>Global</strong> skills are available across all projects.{" "}
          <strong>Project</strong> skills override global ones with the same name.
          Skills are matched by keywords and injected into agent prompts.
        </p>
      </div>
    </div>
  );
}

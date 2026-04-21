import { useCallback, useEffect, useMemo, useState } from "react";
import {
  ChevronDown,
  ChevronRight,
  Edit3,
  FileText,
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
import { Switch } from "@/components/ui/switch";
import {
  type RuleInfo,
  deleteRule,
  listRules,
  readRuleBody,
  saveRule,
} from "@/lib/ai";
import { notify } from "@/lib/notify";

interface EditingRule {
  name: string;
  description: string;
  body: string;
  globs: string;
  alwaysApply: boolean;
  scope: "global" | "project";
  isNew: boolean;
}

function emptyRule(scope: "global" | "project" = "global"): EditingRule {
  return {
    name: "",
    description: "",
    body: "",
    globs: "",
    alwaysApply: false,
    scope,
    isNew: true,
  };
}

export function RulesSettings() {
  const [rules, setRules] = useState<RuleInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [expandedIds, setExpandedIds] = useState<Set<string>>(new Set());
  const [editingRule, setEditingRule] = useState<EditingRule | null>(null);
  const [saving, setSaving] = useState(false);
  const [loadedBodies, setLoadedBodies] = useState<Record<string, string>>({});

  const loadRules = useCallback(async () => {
    try {
      setLoading(true);
      const list = await listRules();
      setRules(list);
    } catch (err) {
      console.error("Failed to load rules:", err);
      notify.error("Failed to load rules");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadRules();
  }, [loadRules]);

  const toggleExpand = async (rule: RuleInfo) => {
    const key = rule.path;
    setExpandedIds((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });

    if (!loadedBodies[key]) {
      try {
        const body = await readRuleBody(rule.path);
        setLoadedBodies((prev) => ({ ...prev, [key]: body }));
      } catch (err) {
        console.error("Failed to load rule body:", err);
      }
    }
  };

  const startEditing = async (rule: RuleInfo) => {
    try {
      const body = loadedBodies[rule.path] || (await readRuleBody(rule.path));
      setEditingRule({
        name: rule.name,
        description: rule.description,
        body,
        globs: rule.globs || "",
        alwaysApply: rule.always_apply,
        scope: rule.source === "project" ? "project" : "global",
        isNew: false,
      });
    } catch (err) {
      console.error("Failed to load rule for editing:", err);
      notify.error("Failed to load rule body");
    }
  };

  const startCreating = (scope: "global" | "project" = "global") => {
    setEditingRule(emptyRule(scope));
  };

  const globalRules = useMemo(() => rules.filter((r) => r.source !== "project"), [rules]);
  const projectRules = useMemo(() => rules.filter((r) => r.source === "project"), [rules]);

  const cancelEditing = () => {
    setEditingRule(null);
  };

  const handleSave = async () => {
    if (!editingRule) return;
    if (!editingRule.name.trim()) {
      notify.error("Rule name is required");
      return;
    }

    try {
      setSaving(true);
      await saveRule({
        name: editingRule.name.trim(),
        description: editingRule.description.trim(),
        body: editingRule.body,
        globs: editingRule.globs || undefined,
        alwaysApply: editingRule.alwaysApply,
        scope: editingRule.scope,
      });
      notify.success(`Rule "${editingRule.name}" saved`);
      setEditingRule(null);
      setLoadedBodies({});
      await loadRules();
    } catch (err) {
      console.error("Failed to save rule:", err);
      notify.error(`Failed to save rule: ${err}`);
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async (rule: RuleInfo) => {
    try {
      await deleteRule(rule.path);
      notify.success(`Rule "${rule.name}" deleted`);
      await loadRules();
    } catch (err) {
      console.error("Failed to delete rule:", err);
      notify.error(`Failed to delete: ${err}`);
    }
  };

  // ── Editing form ──

  if (editingRule) {
    return (
      <div className="space-y-6">
        <div className="flex items-center justify-between">
          <h4 className="text-sm font-medium text-accent">
            {editingRule.isNew ? "Create New Rule" : `Edit: ${editingRule.name}`}
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
              Name {!editingRule.isNew && <span className="text-[10px]">(read-only)</span>}
            </label>
            <Input
              value={editingRule.name}
              onChange={(e) =>
                editingRule.isNew &&
                setEditingRule({
                  ...editingRule,
                  name: e.target.value.replace(/[^a-z0-9_-]/g, ""),
                })
              }
              readOnly={!editingRule.isNew}
              placeholder="my-rule"
              className="bg-background border-border text-foreground h-9 font-mono text-xs"
            />
          </div>
          <div className="space-y-1">
            <label className="text-xs text-muted-foreground">Scope</label>
            <div className="flex gap-1 h-9 items-center">
              <Button
                variant={editingRule.scope === "global" ? "default" : "outline"}
                size="sm"
                onClick={() => setEditingRule({ ...editingRule, scope: "global" })}
                className={`h-8 px-3 text-xs gap-1 ${editingRule.scope === "global" ? "bg-accent text-accent-foreground" : ""}`}
              >
                <Globe className="w-3 h-3" /> Global
              </Button>
              <Button
                variant={editingRule.scope === "project" ? "default" : "outline"}
                size="sm"
                onClick={() => setEditingRule({ ...editingRule, scope: "project" })}
                className={`h-8 px-3 text-xs gap-1 ${editingRule.scope === "project" ? "bg-accent text-accent-foreground" : ""}`}
              >
                <FolderOpen className="w-3 h-3" /> Project
              </Button>
            </div>
          </div>
        </div>

        <div className="space-y-1">
          <label className="text-xs text-muted-foreground">Description</label>
          <Input
            value={editingRule.description}
            onChange={(e) =>
              setEditingRule({ ...editingRule, description: e.target.value })
            }
            placeholder="When and why this rule applies..."
            className="bg-background border-border text-foreground h-9"
          />
        </div>

        <div className="grid grid-cols-2 gap-3">
          <div className="space-y-1">
            <label className="text-xs text-muted-foreground">File Globs</label>
            <Input
              value={editingRule.globs}
              onChange={(e) =>
                setEditingRule({ ...editingRule, globs: e.target.value })
              }
              placeholder="*.ts,*.tsx"
              className="bg-background border-border text-foreground h-9 font-mono text-xs"
            />
            <p className="text-[10px] text-muted-foreground">
              Comma-separated patterns. Leave empty for manual rules.
            </p>
          </div>
          <div className="space-y-1">
            <label className="text-xs text-muted-foreground">Always Apply</label>
            <div className="flex items-center gap-2 pt-1">
              <Switch
                checked={editingRule.alwaysApply}
                onCheckedChange={(v) =>
                  setEditingRule({ ...editingRule, alwaysApply: v })
                }
              />
              <span className="text-xs text-muted-foreground">
                {editingRule.alwaysApply
                  ? "Always injected into prompt"
                  : "Only when matched by globs"}
              </span>
            </div>
          </div>
        </div>

        <div className="space-y-1">
          <label className="text-xs text-muted-foreground">Rule Content (Markdown)</label>
          <Textarea
            value={editingRule.body}
            onChange={(e) =>
              setEditingRule({ ...editingRule, body: e.target.value })
            }
            placeholder="When writing code, always..."
            className="bg-background border-border text-foreground font-mono text-xs min-h-[200px] resize-y"
          />
          <p className="text-[10px] text-muted-foreground">
            {editingRule.body.length} characters
          </p>
        </div>
      </div>
    );
  }

  // ── Rules list ──

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <Loader2 className="w-5 h-5 text-muted-foreground animate-spin" />
      </div>
    );
  }

  const renderRuleCard = (rule: RuleInfo) => {
    const isExpanded = expandedIds.has(rule.path);
    const body = loadedBodies[rule.path];

    return (
      <div
        key={rule.path}
        className="rounded-lg bg-muted border border-[var(--border-medium)] overflow-hidden"
      >
        <button
          type="button"
          onClick={() => toggleExpand(rule)}
          className="w-full flex items-center gap-3 px-4 py-3 text-left hover:bg-[var(--bg-hover)] transition-colors"
        >
          {isExpanded ? (
            <ChevronDown className="w-4 h-4 text-muted-foreground flex-shrink-0" />
          ) : (
            <ChevronRight className="w-4 h-4 text-muted-foreground flex-shrink-0" />
          )}
          <FileText className="w-4 h-4 text-accent flex-shrink-0" />
          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-2">
              <span className="text-sm font-medium text-foreground font-mono">{rule.name}</span>
              {rule.always_apply && (
                <Badge variant="outline" className="text-[9px] px-1.5 py-0 h-4 border-accent text-accent">
                  always
                </Badge>
              )}
              {rule.globs && (
                <span className="text-[10px] font-mono text-muted-foreground/60">{rule.globs}</span>
              )}
            </div>
            <p className="text-xs text-muted-foreground truncate">{rule.description}</p>
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

            <p className="text-[10px] text-muted-foreground/50 font-mono truncate">{rule.path}</p>

            <div className="flex gap-2 pt-1">
              <Button variant="outline" size="sm" onClick={() => startEditing(rule)} className="h-7 text-xs">
                <Edit3 className="w-3 h-3 mr-1" /> Edit
              </Button>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => handleDelete(rule)}
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
      {/* Global Rules */}
      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Globe className="w-4 h-4 text-accent" />
            <h4 className="text-sm font-medium text-accent">Global Rules</h4>
            <span className="text-[10px] text-muted-foreground">~/.golish/rules/</span>
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
          {globalRules.map(renderRuleCard)}
          {globalRules.length === 0 && (
            <p className="text-xs text-muted-foreground italic py-3 text-center">
              No global rules. Click &quot;New&quot; to create one.
            </p>
          )}
        </div>
      </div>

      <div className="border-t border-[var(--border-medium)]" />

      {/* Project Rules */}
      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <FolderOpen className="w-4 h-4 text-accent" />
            <h4 className="text-sm font-medium text-accent">Project Rules</h4>
            <span className="text-[10px] text-muted-foreground">.golish/rules/</span>
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
          {projectRules.map(renderRuleCard)}
          {projectRules.length === 0 && (
            <p className="text-xs text-muted-foreground italic py-3 text-center">
              No project-specific rules. These are stored in your project&apos;s <code>.golish/rules/</code> directory.
            </p>
          )}
        </div>
      </div>

      <div className="text-xs text-muted-foreground border-t border-[var(--border-medium)] pt-4">
        <p>
          <strong>Global</strong> rules apply across all projects.{" "}
          <strong>Project</strong> rules are scoped to the current workspace.
          Rules with &quot;Always Apply&quot; are injected into every prompt.
        </p>
      </div>
    </div>
  );
}

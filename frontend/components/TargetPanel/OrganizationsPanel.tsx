import { Building2, Check, ChevronRight, Pencil, Plus, Trash2, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { organizations as orgsApi } from "@/lib/api";
import type { Organization } from "@/lib/api/organizations";
import { getProjectPath } from "@/lib/projects";
import { cn } from "@/lib/utils";

interface OrgNode extends Organization {
  children: OrgNode[];
  depth: number;
}

function buildTree(rows: Organization[]): OrgNode[] {
  const map = new Map<string, OrgNode>();
  rows.forEach((r) => map.set(r.id, { ...r, children: [], depth: 0 }));

  const roots: OrgNode[] = [];
  rows.forEach((r) => {
    const node = map.get(r.id);
    if (!node) return;
    if (r.parent_id && map.has(r.parent_id)) {
      const parent = map.get(r.parent_id);
      if (parent) {
        node.depth = parent.depth + 1;
        parent.children.push(node);
      }
    } else {
      roots.push(node);
    }
  });

  const recomputeDepth = (n: OrgNode, d: number) => {
    n.depth = d;
    n.children.forEach((c) => recomputeDepth(c, d + 1));
  };
  roots.forEach((r) => recomputeDepth(r, 0));

  const sortRec = (list: OrgNode[]) => {
    list.sort((a, b) => a.sort_order - b.sort_order || a.name.localeCompare(b.name, "zh"));
    list.forEach((c) => sortRec(c.children));
  };
  sortRec(roots);
  return roots;
}

function flattenWithDepth(nodes: OrgNode[]): OrgNode[] {
  const out: OrgNode[] = [];
  const walk = (list: OrgNode[]) => {
    list.forEach((n) => {
      out.push(n);
      if (n.children.length > 0) walk(n.children);
    });
  };
  walk(nodes);
  return out;
}

export function OrganizationsPanel() {
  const { t } = useTranslation();
  const [rows, setRows] = useState<Organization[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const [newName, setNewName] = useState("");
  const [newParent, setNewParent] = useState<string>("");
  const [newOwner, setNewOwner] = useState("");
  const [creating, setCreating] = useState(false);

  const [editingId, setEditingId] = useState<string | null>(null);
  const [editName, setEditName] = useState("");
  const [editOwner, setEditOwner] = useState("");
  const [editDesc, setEditDesc] = useState("");

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const list = await orgsApi.listOrganizations(getProjectPath());
      setRows(list);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const tree = useMemo(() => buildTree(rows), [rows]);
  const flat = useMemo(() => flattenWithDepth(tree), [tree]);

  const handleCreate = useCallback(async () => {
    if (!newName.trim()) return;
    setCreating(true);
    try {
      await orgsApi.createOrganization({
        projectPath: getProjectPath(),
        name: newName.trim(),
        parentId: newParent || undefined,
        owner: newOwner.trim() || undefined,
      });
      setNewName("");
      setNewParent("");
      setNewOwner("");
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setCreating(false);
    }
  }, [newName, newParent, newOwner, refresh]);

  const handleSaveEdit = useCallback(async () => {
    if (!editingId) return;
    try {
      await orgsApi.updateOrganization({
        id: editingId,
        name: editName.trim() || undefined,
        owner: editOwner.trim(),
        description: editDesc.trim(),
      });
      setEditingId(null);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  }, [editingId, editName, editOwner, editDesc, refresh]);

  const handleDelete = useCallback(
    async (id: string, name: string) => {
      if (!confirm(t("organizations.deleteConfirm", { name }))) return;
      try {
        await orgsApi.deleteOrganization(id);
        await refresh();
      } catch (e) {
        setError(String(e));
      }
    },
    [refresh, t]
  );

  return (
    <div className="flex flex-col h-full">
      {/* New org form */}
      <div className="px-4 py-3 border-b border-border/30 bg-muted/10 space-y-2">
        <div className="flex items-center gap-2">
          <Plus className="w-3.5 h-3.5 text-accent" />
          <span className="text-xs font-medium">{t("organizations.create")}</span>
        </div>
        <div className="flex items-center gap-2">
          <input
            className="flex-1 text-xs bg-background border border-border/50 rounded px-2 py-1.5 outline-none focus:border-accent"
            placeholder={t("organizations.namePlaceholder")}
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") handleCreate();
            }}
          />
          <select
            className="text-xs bg-background border border-border/50 rounded px-2 py-1.5 outline-none focus:border-accent max-w-[160px]"
            value={newParent}
            onChange={(e) => setNewParent(e.target.value)}
          >
            <option value="">{t("organizations.noParent")}</option>
            {flat.map((n) => (
              <option key={n.id} value={n.id}>
                {"  ".repeat(n.depth)}
                {n.name}
              </option>
            ))}
          </select>
          <input
            className="text-xs bg-background border border-border/50 rounded px-2 py-1.5 outline-none focus:border-accent w-32"
            placeholder={t("organizations.ownerPlaceholder")}
            value={newOwner}
            onChange={(e) => setNewOwner(e.target.value)}
          />
          <button
            type="button"
            className={cn(
              "px-3 py-1 text-xs rounded bg-accent text-accent-foreground hover:bg-accent/90",
              (!newName.trim() || creating) && "opacity-50"
            )}
            disabled={!newName.trim() || creating}
            onClick={handleCreate}
          >
            {t("organizations.add")}
          </button>
        </div>
      </div>

      {/* Error banner */}
      {error && (
        <div className="px-4 py-2 text-[11px] text-red-400 bg-red-500/10 border-b border-red-500/20">
          {error}
        </div>
      )}

      {/* Tree list */}
      <div className="flex-1 overflow-y-auto">
        {loading ? (
          <div className="flex items-center justify-center h-full text-muted-foreground text-xs">
            {t("organizations.loading")}
          </div>
        ) : flat.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-full text-muted-foreground">
            <Building2 className="w-8 h-8 mb-2 opacity-30" />
            <p className="text-xs">{t("organizations.empty")}</p>
          </div>
        ) : (
          <ul className="divide-y divide-border/20">
            {flat.map((node) => (
              <li
                key={node.id}
                className="px-4 py-2 hover:bg-muted/20 transition-colors group"
                style={{ paddingLeft: `${16 + node.depth * 20}px` }}
              >
                <div className="flex items-center gap-2">
                  {node.depth > 0 && (
                    <ChevronRight className="w-3 h-3 text-muted-foreground/40" />
                  )}
                  <Building2 className="w-3.5 h-3.5 text-accent/70" />
                  {editingId === node.id ? (
                    <>
                      <input
                        className="flex-1 text-xs bg-background border border-border/50 rounded px-2 py-1 outline-none focus:border-accent"
                        value={editName}
                        onChange={(e) => setEditName(e.target.value)}
                      />
                      <input
                        className="text-xs bg-background border border-border/50 rounded px-2 py-1 outline-none focus:border-accent w-28"
                        placeholder={t("organizations.ownerPlaceholder")}
                        value={editOwner}
                        onChange={(e) => setEditOwner(e.target.value)}
                      />
                      <button
                        type="button"
                        className="p-1 rounded text-green-400 hover:bg-green-500/10"
                        onClick={handleSaveEdit}
                      >
                        <Check className="w-3 h-3" />
                      </button>
                      <button
                        type="button"
                        className="p-1 rounded text-muted-foreground hover:text-foreground"
                        onClick={() => setEditingId(null)}
                      >
                        <X className="w-3 h-3" />
                      </button>
                    </>
                  ) : (
                    <>
                      <span className="text-xs font-medium text-foreground flex-1 truncate">
                        {node.name}
                      </span>
                      {node.owner && (
                        <span className="text-[10px] text-muted-foreground">
                          {t("organizations.ownedBy", { owner: node.owner })}
                        </span>
                      )}
                      <button
                        type="button"
                        className="p-1 rounded opacity-0 group-hover:opacity-100 hover:bg-muted/50 text-muted-foreground hover:text-foreground transition-all"
                        onClick={() => {
                          setEditingId(node.id);
                          setEditName(node.name);
                          setEditOwner(node.owner);
                          setEditDesc(node.description);
                        }}
                      >
                        <Pencil className="w-3 h-3" />
                      </button>
                      <button
                        type="button"
                        className="p-1 rounded opacity-0 group-hover:opacity-100 hover:bg-red-500/20 text-muted-foreground hover:text-red-400 transition-all"
                        onClick={() => handleDelete(node.id, node.name)}
                      >
                        <Trash2 className="w-3 h-3" />
                      </button>
                    </>
                  )}
                </div>
                {node.description && editingId !== node.id && (
                  <p className="text-[10px] text-muted-foreground/70 mt-0.5 ml-5 truncate">
                    {node.description}
                  </p>
                )}
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

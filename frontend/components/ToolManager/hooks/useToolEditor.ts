import { useCallback, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import type { ToolWithMeta } from "../OutputParserEditor";
import { listSkills, type SkillFileInfo } from "@/lib/pentest/api";
import {
  RUNTIME_VERSION_MAP,
  VALID_RUNTIMES, VALID_LAUNCH_MODES, VALID_TIERS, VALID_INSTALL_METHODS, VALID_PARAM_TYPES,
} from "../EditorFields";

function generateToolId(): string {
  const u = (globalThis.crypto as Crypto | undefined)?.randomUUID?.();
  if (u) return u.replace(/-/g, "").slice(0, 8);
  return Math.random().toString(36).slice(2, 10);
}

export function useToolEditor(loadData: (silent?: boolean) => Promise<void>, setError: (err: string | null) => void) {
  const { t } = useTranslation();
  const [editingTool, setEditingTool] = useState<ToolWithMeta | null>(null);
  const [editorMode, setEditorMode] = useState<"form" | "raw" | "skills" | "output">("form");
  const [rawJson, setRawJson] = useState("");
  const [formData, setFormData] = useState<Record<string, unknown>>({});
  const [editorLoading, setEditorLoading] = useState(false);
  const [editorDirty, setEditorDirty] = useState(false);
  const [saving, setSaving] = useState(false);
  const [showCloseConfirm, setShowCloseConfirm] = useState(false);
  const [editorVisible, setEditorVisible] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const originalJsonRef = useRef("");

  // Skills state (co-located since it's tied to the editor lifecycle)
  const [skillsList, setSkillsList] = useState<SkillFileInfo[]>([]);
  const [skillDirty, setSkillDirty] = useState(false);

  const syncFormToRaw = useCallback((data: Record<string, unknown>) => {
    setRawJson(JSON.stringify({ tool: data }, null, 2));
  }, []);
  const syncRawToForm = useCallback((json: string) => {
    try {
      const p = JSON.parse(json) as { tool?: Record<string, unknown> } | Record<string, unknown>;
      const next = ("tool" in p && p.tool ? p.tool : p) as Record<string, unknown>;
      setFormData(next);
    } catch { /* ignore */ }
  }, []);

  const handleFormChange = useCallback((field: string, value: unknown) => {
    setFormData((prev) => {
      if (JSON.stringify(prev[field]) === JSON.stringify(value)) return prev;
      const next = { ...prev, [field]: value };

      if (field === "runtime") {
        const rt = value as string;
        const oldVersion = (prev.runtimeVersion as string) || "";
        if (rt === "native") {
          next.runtimeVersion = "";
        } else {
          const allowed = RUNTIME_VERSION_MAP[rt];
          if (allowed && oldVersion && !allowed.some((v) => v.value === oldVersion)) {
            next.runtimeVersion = allowed[allowed.length - 1]?.value ?? "";
          }
        }
      }

      syncFormToRaw(next);
      setEditorDirty(true);
      return next;
    });
  }, [syncFormToRaw]);

  const handleRawChange = useCallback((value: string) => {
    setRawJson(value);
    setEditorDirty(true);
    syncRawToForm(value);
  }, [syncRawToForm]);

  const openEditor = useCallback(async (tool: ToolWithMeta) => {
    setEditingTool(tool);
    setEditorMode("form");
    setEditorDirty(false);
    setEditorLoading(true);
    requestAnimationFrame(() => setEditorVisible(true));
    try {
      const content: string = await invoke("pentest_read_tool_config", {
        toolId: tool.id,
      });
      originalJsonRef.current = content;
      setRawJson(content);
      const parsed = JSON.parse(content) as { tool?: Record<string, unknown> };
      const toolObj = parsed.tool || (parsed as unknown as Record<string, unknown>);
      setFormData(toolObj as Record<string, unknown>);
    } catch {
      const fallback: Record<string, unknown> = {};
      const RUNTIME_FIELDS = new Set(["categoryName", "subcategoryName", "installed", "envReady", "installedVia", "executableReady", "executableError"]);
      for (const [k, v] of Object.entries(tool)) {
        if (!RUNTIME_FIELDS.has(k)) fallback[k] = v;
      }
      const json = JSON.stringify({ tool: fallback }, null, 2);
      originalJsonRef.current = json;
      setRawJson(json);
      setFormData(fallback);
    } finally {
      setEditorLoading(false);
    }
  }, []);

  const animateClose = useCallback(() => {
    setEditorVisible(false);
    setTimeout(() => { setEditingTool(null); setEditorDirty(false); }, 180);
  }, []);

  const closeEditor = useCallback(() => {
    if (editorDirty || skillDirty) { setShowCloseConfirm(true); return; }
    animateClose();
  }, [editorDirty, skillDirty, animateClose]);

  const forceCloseEditor = useCallback(() => {
    setShowCloseConfirm(false);
    animateClose();
  }, [animateClose]);

  const validateToolData = useCallback((data: Record<string, unknown>): string[] => {
    const errors: string[] = [];
    if (!data.name || !(data.name as string).trim()) errors.push(t("toolManager.validationNameRequired", "Name is required"));
    if (!data.id || !(data.id as string).trim()) errors.push(t("toolManager.validationIdRequired", "ID is required"));
    if (!data.executable || !(data.executable as string).trim()) errors.push(t("toolManager.validationExecRequired", "Executable is required"));

    const rt = ((data.runtime as string) || "").trim().toLowerCase();
    if (rt && !(VALID_RUNTIMES as readonly string[]).includes(rt)) {
      errors.push(t("toolManager.validationRuntime", { value: rt, allowed: VALID_RUNTIMES.join(", ") }));
    }
    const lm = ((data.launchMode as string) || "").trim().toLowerCase();
    if (lm && !(VALID_LAUNCH_MODES as readonly string[]).includes(lm)) {
      errors.push(t("toolManager.validationLaunchMode", { value: lm, allowed: VALID_LAUNCH_MODES.join(", ") }));
    }
    const tier = ((data.tier as string) || "").trim().toLowerCase();
    if (tier && !(VALID_TIERS as readonly string[]).includes(tier)) {
      errors.push(t("toolManager.validationTier", { value: tier, allowed: VALID_TIERS.join(", ") }));
    }

    const install = data.install as Record<string, string> | undefined;
    if (install?.method) {
      const m = install.method.trim().toLowerCase();
      if (!(VALID_INSTALL_METHODS as readonly string[]).includes(m)) {
        errors.push(t("toolManager.validationInstallMethod", { value: m, allowed: VALID_INSTALL_METHODS.filter(Boolean).join(", ") }));
      }
    }

    const rtVersion = ((data.runtimeVersion as string) || "").trim();
    if (rt && rt !== "native" && rtVersion) {
      const allowed = RUNTIME_VERSION_MAP[rt];
      if (allowed && !allowed.some((v) => v.value === rtVersion)) {
        errors.push(t("toolManager.validationRuntimeVersion", {
          value: rtVersion, runtime: rt, allowed: allowed.map((v) => v.value).join(", "),
        }));
      }
    }

    const params = data.params as Array<Record<string, unknown>> | undefined;
    if (params && Array.isArray(params)) {
      for (let i = 0; i < params.length; i++) {
        const pt = ((params[i].type as string) || "").trim().toLowerCase();
        if (pt && !(VALID_PARAM_TYPES as readonly string[]).includes(pt)) {
          errors.push(t("toolManager.validationParamType", { index: i + 1, value: pt, allowed: VALID_PARAM_TYPES.join(", ") }));
        }
      }
    }

    return errors;
  }, [t]);

  const handleSave = useCallback(async () => {
    if (!editingTool) return;
    setSaving(true);
    setError(null);
    try {
      let data: Record<string, unknown>;
      if (editorMode === "raw") {
        const parsed = JSON.parse(rawJson);
        data = parsed.tool || parsed;
      } else {
        data = formData;
      }

      const validationErrors = validateToolData(data);
      if (validationErrors.length > 0) {
        setError(validationErrors.join(" · "));
        setSaving(false);
        return;
      }

      const KNOWN_FIELDS = new Set([
        "id", "name", "description", "executable", "runtime", "launchMode", "args",
        "version", "tags", "category", "subcategory", "jvm_options",
        "icon",
        "runtimeVersion", "install", "skills", "output", "tier", "params",
      ]);
      const unknownFields = Object.keys(data).filter((k) => !KNOWN_FIELDS.has(k));
      if (unknownFields.length > 0) {
        const msg = t("toolManager.validationUnknownFields", {
          fields: unknownFields.join(", "),
          defaultValue: `Unknown fields will be ignored: ${unknownFields.join(", ")}`,
        });
        console.warn("[ToolEditor]", msg);
      }

      const content = JSON.stringify({ tool: data }, null, 2);
      const toolId = (data.id as string) || editingTool.id;

      await invoke("pentest_save_tool_config", { toolId, content });
      setEditorDirty(false);
      if (editorMode === "raw") {
        setRawJson(content);
        setFormData(data);
      }
      await loadData(true);
    } catch (e) {
      setError(t("toolManager.saveFailed", { error: e }));
    } finally {
      setSaving(false);
    }
  }, [editingTool, editorMode, rawJson, formData, loadData, setError, t, validateToolData]);

  const handleSwitchMode = useCallback((mode: "form" | "raw" | "skills" | "output") => {
    if (mode === "skills") {
      setEditorMode("skills");
      if (editingTool) {
        listSkills(editingTool.name).then(setSkillsList).catch(() => setSkillsList([]));
      }
      return;
    }
    if (mode === "output") { syncRawToForm(rawJson); setEditorMode("output"); return; }
    if (mode === "raw") syncFormToRaw(formData);
    else syncRawToForm(rawJson);
    setEditorMode(mode);
  }, [formData, rawJson, syncFormToRaw, syncRawToForm, editingTool]);

  const handleAddTool = useCallback(() => {
    const id = generateToolId();
    const defaults: Record<string, unknown> = {
      id, name: "", description: "", icon: "🔧", executable: "",
      runtime: "native", runtimeVersion: "", launchMode: "cli", params: [],
      install: { method: "", source: "" },
    };
    const json = JSON.stringify({ tool: defaults }, null, 2);
    const placeholder: ToolWithMeta = {
      ...defaults, name: t("toolManager.newTool"),
      category: "misc", subcategory: "other", installed: false,
      categoryName: "misc", subcategoryName: "other",
    } as unknown as ToolWithMeta;
    setEditingTool(placeholder);
    setEditorMode("form");
    setEditorDirty(true);
    setEditorLoading(false);
    originalJsonRef.current = json;
    setRawJson(json);
    setFormData(defaults);
    requestAnimationFrame(() => setEditorVisible(true));
  }, [t]);

  return {
    editingTool, setEditingTool, editorMode, editorVisible,
    rawJson, formData, editorLoading, editorDirty, saving,
    showCloseConfirm, setShowCloseConfirm, textareaRef,
    skillsList, setSkillsList, skillDirty, setSkillDirty,
    syncFormToRaw,
    handleFormChange, handleRawChange, openEditor,
    closeEditor, forceCloseEditor, handleSave, handleSwitchMode, handleAddTool,
  };
}

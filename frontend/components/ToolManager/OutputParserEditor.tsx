import { useCallback, useState } from "react";
import {
  Trash2, X,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { invoke } from "@tauri-apps/api/core";
import type { ToolConfig } from "@/lib/pentest/types";
export type ToolWithMeta = ToolConfig & { categoryName?: string; subcategoryName?: string };
export type ViewMode = "grid" | "list";
export type SortKey = "name" | "status" | "category" | "runtime";

export interface OutputPattern {
  type: string;
  regex: string;
  fields: Record<string, string>;
}

export interface OutputConfigData {
  format: string;
  produces: string[];
  detect?: string;
  patterns: OutputPattern[];
  fields: Record<string, string>;
  db_action?: string;
  transform?: string;
}

export const PRODUCE_TYPES = ["host", "port", "vulnerability", "url", "credential"];
export const OUTPUT_FORMATS = [
  { value: "text", label: "Text (Regex)" },
  { value: "json_lines", label: "JSON Lines" },
  { value: "json", label: "JSON" },
];
export const DB_ACTIONS = [
  { value: "", label: "None (don't store)" },
  { value: "target_add", label: "Add Target" },
  { value: "target_update_recon", label: "Update Target Recon" },
  { value: "directory_entry_add", label: "Add Directory Entry" },
  { value: "finding_add", label: "Add Finding" },
];

import { MiniDropdown } from "@/components/ui/MiniDropdown";
export const OutputMiniDropdown = (props: { value: string; onChange: (v: string) => void; options: { value: string; label: string }[] }) => (
  <MiniDropdown variant="standard" {...props} />
);

export function OutputParserEditor({
  formData,
  onChange,
}: {
  formData: Record<string, unknown>;
  onChange: (output: OutputConfigData) => void;
}) {
  const existing = (formData.output as OutputConfigData | undefined) || {
    format: "text",
    produces: [],
    detect: "",
    patterns: [],
    fields: {},
    db_action: undefined,
    transform: undefined,
  };

  const [config, setConfig] = useState<OutputConfigData>(existing);
  const [testInput, setTestInput] = useState("");
  const [testResult, setTestResult] = useState<string | null>(null);

  const update = useCallback((patch: Partial<OutputConfigData>) => {
    const next = { ...config, ...patch };
    setConfig(next);
    onChange(next);
  }, [config, onChange]);

  const toggleProduce = useCallback((type: string) => {
    const produces = config.produces.includes(type)
      ? config.produces.filter((t) => t !== type)
      : [...config.produces, type];
    update({ produces });
  }, [config.produces, update]);

  const addPattern = useCallback(() => {
    update({
      patterns: [...config.patterns, { type: "host", regex: "", fields: {} }],
    });
  }, [config.patterns, update]);

  const removePattern = useCallback((idx: number) => {
    update({ patterns: config.patterns.filter((_, i) => i !== idx) });
  }, [config.patterns, update]);

  const updatePattern = useCallback((idx: number, patch: Partial<OutputPattern>) => {
    const patterns = config.patterns.map((p, i) =>
      i === idx ? { ...p, ...patch } : p
    );
    update({ patterns });
  }, [config.patterns, update]);

  const addField = useCallback(() => {
    const fields = { ...config.fields, "": "" };
    update({ fields });
  }, [config.fields, update]);

  const handleTestParse = useCallback(async () => {
    if (!testInput.trim()) return;
    try {
      const result = await invoke<{ items: { data_type: string; fields: Record<string, string> }[] }>("output_parse", {
        rawOutput: testInput,
        config,
        toolId: formData.id || null,
        toolName: formData.name || null,
      });
      setTestResult(JSON.stringify(result.items, null, 2));
    } catch (e) {
      setTestResult(`Error: ${e}`);
    }
  }, [testInput, config, formData]);

  return (
    <div className="flex gap-4 h-full min-h-[400px]">
      <div className="flex-1 min-w-0 space-y-4 overflow-y-auto">
        {/* Format & Detection */}
        <div className="rounded-xl bg-[var(--bg-hover)]/20 overflow-hidden">
          <div className="px-3 py-2 border-b border-border/8">
            <span className="text-[11px] font-medium text-muted-foreground/40">Output Format</span>
          </div>
          <div className="p-3 space-y-3">
            <div className="flex items-center gap-3">
              <label className="text-[11px] text-muted-foreground/50 w-20 flex-shrink-0">Format</label>
              <OutputMiniDropdown
                value={config.format}
                onChange={(v) => update({ format: v })}
                options={OUTPUT_FORMATS}
              />
            </div>
            <div className="flex items-center gap-3">
              <label className="text-[11px] text-muted-foreground/50 w-20 flex-shrink-0">Detect</label>
              <input
                value={config.detect || ""}
                onChange={(e) => update({ detect: e.target.value })}
                placeholder="Regex to match command or output"
                className="flex-1 px-2 py-1 text-[11px] font-mono rounded-md bg-transparent border border-border/20 text-foreground placeholder:text-muted-foreground/20 outline-none"
              />
            </div>
            <div className="flex items-center gap-3">
              <label className="text-[11px] text-muted-foreground/50 w-20 flex-shrink-0">DB Action</label>
              <OutputMiniDropdown
                value={config.db_action || ""}
                onChange={(v) => update({ db_action: v || undefined })}
                options={DB_ACTIONS}
              />
            </div>
            <div className="flex items-center gap-3">
              <label className="text-[11px] text-muted-foreground/50 w-20 flex-shrink-0">Transform</label>
              <input
                value={config.transform || ""}
                onChange={(e) => update({ transform: e.target.value || undefined })}
                placeholder="jq expression to pre-process output (e.g. '.[] | .plugins | to_entries[]')"
                className="flex-1 px-2 py-1 text-[11px] font-mono rounded-md bg-transparent border border-border/20 text-foreground placeholder:text-muted-foreground/20 outline-none"
              />
            </div>
          </div>
        </div>

        {/* Produces */}
        <div className="rounded-xl bg-[var(--bg-hover)]/20 overflow-hidden">
          <div className="px-3 py-2 border-b border-border/8">
            <span className="text-[11px] font-medium text-muted-foreground/40">Produces</span>
          </div>
          <div className="p-3 flex flex-wrap gap-2">
            {PRODUCE_TYPES.map((type) => (
              <button
                key={type}
                type="button"
                onClick={() => toggleProduce(type)}
                className={cn(
                  "px-2.5 py-1 rounded-md text-[10px] font-medium transition-colors",
                  config.produces.includes(type)
                    ? "bg-accent/15 text-accent border border-accent/30"
                    : "bg-muted/10 text-muted-foreground/40 border border-border/10 hover:border-border/30"
                )}
              >
                {type}
              </button>
            ))}
          </div>
        </div>

        {/* Patterns (for text format) */}
        {config.format === "text" && (
          <div className="rounded-xl bg-[var(--bg-hover)]/20 overflow-hidden">
            <div className="px-3 py-2 border-b border-border/8 flex items-center justify-between">
              <span className="text-[11px] font-medium text-muted-foreground/40">Regex Patterns</span>
              <button type="button" onClick={addPattern}
                className="text-[10px] text-accent/70 hover:text-accent transition-colors">
                + Add Pattern
              </button>
            </div>
            <div className="p-3 space-y-3">
              {config.patterns.map((pattern, idx) => (
                <div key={idx} className="rounded-lg border border-border/10 p-2.5 space-y-2">
                  <div className="flex items-center justify-between">
                    <div className="flex items-center gap-2">
                      <OutputMiniDropdown
                        value={pattern.type}
                        onChange={(v) => updatePattern(idx, { type: v })}
                        options={PRODUCE_TYPES.map((t) => ({ value: t, label: t }))}
                      />
                    </div>
                    <button type="button" onClick={() => removePattern(idx)}
                      className="p-0.5 text-muted-foreground/30 hover:text-red-400 transition-colors">
                      <Trash2 className="w-3 h-3" />
                    </button>
                  </div>
                  <input
                    value={pattern.regex}
                    onChange={(e) => updatePattern(idx, { regex: e.target.value })}
                    placeholder="Regular expression with capture groups"
                    className="w-full px-2 py-1 text-[11px] font-mono rounded-md bg-transparent border border-border/20 text-foreground placeholder:text-muted-foreground/20 outline-none"
                  />
                  <div className="text-[9px] text-muted-foreground/50 px-1">
                    Fields: {Object.entries(pattern.fields).map(([k, v]) => `${k}=${v}`).join(", ") || "none"}
                  </div>
                  <input
                    value={Object.entries(pattern.fields).map(([k, v]) => `${k}=${v}`).join(", ")}
                    onChange={(e) => {
                      const fields: Record<string, string> = {};
                      for (const pair of e.target.value.split(",")) {
                        const [k, v] = pair.split("=").map((s) => s.trim());
                        if (k && v) fields[k] = v;
                      }
                      updatePattern(idx, { fields });
                    }}
                    placeholder='field=$1, field2=$2'
                    className="w-full px-2 py-1 text-[10px] font-mono rounded-md bg-transparent border border-border/15 text-foreground placeholder:text-muted-foreground/20 outline-none"
                  />
                </div>
              ))}
              {config.patterns.length === 0 && (
                <div className="text-center text-[10px] text-muted-foreground/50 py-2">
                  No patterns defined. Click &quot;+ Add Pattern&quot; to create one.
                </div>
              )}
            </div>
          </div>
        )}

        {/* Fields (for JSON format) */}
        {(config.format === "json" || config.format === "json_lines") && (
          <div className="rounded-xl bg-[var(--bg-hover)]/20 overflow-hidden">
            <div className="px-3 py-2 border-b border-border/8 flex items-center justify-between">
              <span className="text-[11px] font-medium text-muted-foreground/60">JSON Field Mappings</span>
              <button type="button" onClick={addField}
                className="text-[10px] text-accent/70 hover:text-accent transition-colors">
                + Add Field
              </button>
            </div>
            <div className="p-3 space-y-2">
              {Object.entries(config.fields).map(([key, path], idx) => (
                <div key={idx} className="flex items-center gap-2">
                  <input
                    value={key}
                    onChange={(e) => {
                      const newFields = { ...config.fields };
                      delete newFields[key];
                      newFields[e.target.value] = path;
                      update({ fields: newFields });
                    }}
                    placeholder="field_name"
                    className="w-28 px-2 py-1 text-[10px] font-mono rounded-md bg-transparent border border-border/20 text-foreground outline-none"
                  />
                  <span className="text-[10px] text-muted-foreground/50">→</span>
                  <input
                    value={path}
                    onChange={(e) => {
                      const newFields = { ...config.fields };
                      newFields[key] = e.target.value;
                      update({ fields: newFields });
                    }}
                    placeholder="$.json.path"
                    className="flex-1 px-2 py-1 text-[10px] font-mono rounded-md bg-transparent border border-border/20 text-foreground outline-none"
                  />
                  <button type="button" onClick={() => {
                    const newFields = { ...config.fields };
                    delete newFields[key];
                    update({ fields: newFields });
                  }}
                    className="p-0.5 text-muted-foreground/30 hover:text-red-400">
                    <X className="w-3 h-3" />
                  </button>
                </div>
              ))}
            </div>
          </div>
        )}
      </div>

      {/* Test panel */}
      <div className="w-[300px] flex-shrink-0 flex flex-col rounded-xl bg-[var(--bg-hover)]/20 overflow-hidden">
        <div className="px-3 py-2 border-b border-border/8 flex items-center justify-between">
          <span className="text-[11px] font-medium text-muted-foreground/40">Test Parser</span>
          <button type="button" onClick={handleTestParse}
            className="text-[10px] px-2 py-0.5 rounded bg-accent/15 text-accent hover:bg-accent/25 transition-colors">
            Parse
          </button>
        </div>
        <textarea
          value={testInput}
          onChange={(e) => setTestInput(e.target.value)}
          placeholder="Paste tool output here to test parsing..."
          className="flex-1 px-3 py-2 text-[10px] font-mono leading-[1.6] bg-transparent text-foreground outline-none resize-none border-b border-border/8"
          style={{ tabSize: 2 }}
        />
        {testResult && (
          <div className="max-h-[200px] overflow-y-auto px-3 py-2">
            <pre className="text-[9px] font-mono text-emerald-400/70 whitespace-pre-wrap">{testResult}</pre>
          </div>
        )}
      </div>
    </div>
  );
}


import { Plus, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "@/lib/utils";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";

export interface EditorFieldsContext {
  formData: Record<string, unknown>;
  handleFormChange: (field: string, value: unknown) => void;
}

function InlineSelect({ value, onChange, options }: {
  value: string; onChange: (v: string) => void;
  options: { value: string; label: string }[];
}) {
  const { t } = useTranslation();
  return (
    <Select value={value || undefined} onValueChange={onChange}>
      <SelectTrigger size="sm" className="flex-1 h-7 border-transparent bg-transparent hover:border-border/20 text-[12px] shadow-none px-2 gap-1">
        <SelectValue placeholder={t("common.select")} />
      </SelectTrigger>
      <SelectContent position="popper" className="min-w-[120px]">
        {options.map((o) => <SelectItem key={o.value} value={o.value || "_none"} className="text-[12px]">{o.label}</SelectItem>)}
      </SelectContent>
    </Select>
  );
}

export function FieldRow({ label, field, placeholder, mono, type = "text", options, ctx }: {
  label: string; field: string; placeholder?: string; mono?: boolean;
  type?: "text" | "select"; options?: { value: string; label: string }[];
  ctx: EditorFieldsContext;
}) {
  const val = (ctx.formData[field] as string) ?? "";
  return (
    <div className="flex items-center gap-3 py-2 px-3 rounded-lg hover:bg-[var(--bg-hover)]/30 transition-colors">
      <span className="text-[12px] text-muted-foreground/60 w-24 flex-shrink-0">{label}</span>
      {type === "select" && options ? (
        <InlineSelect value={val} onChange={(v) => ctx.handleFormChange(field, v === "_none" ? "" : v)} options={options} />
      ) : (
        <input type="text" value={val} onChange={(e) => ctx.handleFormChange(field, e.target.value)} placeholder={placeholder}
          className={cn("flex-1 h-7 px-2 text-[12px] rounded-md bg-transparent border border-transparent hover:border-border/20 focus:border-accent/40 text-foreground placeholder:text-muted-foreground/20 outline-none transition-colors", mono && "font-mono text-[11px]")} />
      )}
    </div>
  );
}

export function InstallFieldRow({ label, subField, placeholder, mono, type = "text", options, ctx }: {
  label: string; subField: string; placeholder?: string; mono?: boolean;
  type?: "text" | "select"; options?: { value: string; label: string }[];
  ctx: EditorFieldsContext;
}) {
  const install = (ctx.formData.install as Record<string, string>) || {};
  const val = install[subField] || "";
  const onChange = (v: string) => ctx.handleFormChange("install", { ...install, [subField]: v === "_none" ? "" : v });
  return (
    <div className="flex items-center gap-3 py-2 px-3 rounded-lg hover:bg-[var(--bg-hover)]/30 transition-colors">
      <span className="text-[12px] text-muted-foreground/60 w-24 flex-shrink-0">{label}</span>
      {type === "select" && options ? (
        <InlineSelect value={val} onChange={onChange} options={options} />
      ) : (
        <input type="text" value={val} onChange={(e) => onChange(e.target.value)} placeholder={placeholder}
          className={cn("flex-1 h-7 px-2 text-[12px] rounded-md bg-transparent border border-transparent hover:border-border/20 focus:border-accent/40 text-foreground placeholder:text-muted-foreground/20 outline-none transition-colors", mono && "font-mono text-[11px]")} />
      )}
    </div>
  );
}

export function ParamsEditor({ ctx }: { ctx: EditorFieldsContext }) {
  const { t } = useTranslation();
  const params = (ctx.formData.params as Array<Record<string, unknown>>) || [];
  const updateParam = (idx: number, key: string, value: unknown) => {
    const next = [...params]; next[idx] = { ...next[idx], [key]: value };
    ctx.handleFormChange("params", next);
  };
  const removeParam = (idx: number) => ctx.handleFormChange("params", params.filter((_, i) => i !== idx));
  const addParam = () => ctx.handleFormChange("params", [...params, { label: "", flag: "", type: "string" }]);

  return (
    <div className="px-3">
      {params.length === 0 ? (
        <p className="text-[12px] text-muted-foreground/50 py-3 text-center">{t("toolManager.noParams")}</p>
      ) : (
        <div className="space-y-1">
          {params.map((p, i) => (
            <div key={i} className="flex items-center gap-2 py-1.5 group/param rounded-lg hover:bg-[var(--bg-hover)]/30 px-1">
              <input value={(p.label as string) || ""} onChange={(e) => updateParam(i, "label", e.target.value)}
                placeholder={t("toolManager.label")} className="flex-[3] h-7 px-2 text-[12px] rounded-md bg-transparent border border-transparent hover:border-border/20 focus:border-accent/40 text-foreground placeholder:text-muted-foreground/20 outline-none transition-colors" />
              <input value={(p.flag as string) || ""} onChange={(e) => updateParam(i, "flag", e.target.value)}
                placeholder="--flag" className="flex-[2] h-7 px-2 text-[11px] font-mono rounded-md bg-transparent border border-transparent hover:border-border/20 focus:border-accent/40 text-foreground placeholder:text-muted-foreground/20 outline-none transition-colors" />
              <div className="flex-[1.5]">
                <Select value={(p.type as string) || "string"} onValueChange={(v) => updateParam(i, "type", v)}>
                  <SelectTrigger size="sm" className="h-7 border-transparent bg-transparent hover:border-border/20 text-[11px] shadow-none px-2 gap-1 w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent position="popper" className="min-w-[100px]">
                    {["string", "number", "boolean", "file"].map((tp) => <SelectItem key={tp} value={tp} className="text-[12px]">{tp}</SelectItem>)}
                  </SelectContent>
                </Select>
              </div>
              <button type="button" onClick={() => removeParam(i)}
                className="p-0.5 text-muted-foreground/15 opacity-0 group-hover/param:opacity-100 hover:text-destructive transition-all flex-shrink-0">
                <X className="w-3 h-3" />
              </button>
            </div>
          ))}
        </div>
      )}
      <button type="button" onClick={addParam}
        className="flex items-center gap-1 text-[11px] text-accent/60 hover:text-accent transition-colors mt-2 px-1">
        <Plus className="w-3 h-3" /> {t("toolManager.addParam")}
      </button>
    </div>
  );
}

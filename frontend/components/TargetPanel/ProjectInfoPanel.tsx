import { Calendar, ChevronDown, Pencil, Save, Users, X } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { engagements } from "@/lib/api";
import type { Engagement } from "@/lib/api/engagements";
import { getProjectPath } from "@/lib/projects";
import { cn } from "@/lib/utils";

function unixToInputValue(ts: number | null | undefined): string {
  if (!ts) return "";
  const d = new Date(ts * 1000);
  if (Number.isNaN(d.getTime())) return "";
  const off = d.getTimezoneOffset() * 60_000;
  return new Date(d.getTime() - off).toISOString().slice(0, 16);
}

function inputToIso(local: string): string {
  if (!local) return "";
  const d = new Date(local);
  return Number.isNaN(d.getTime()) ? "" : d.toISOString();
}

function formatRemaining(now: number, startTs: number | null, endTs: number | null): string {
  if (!endTs) return "";
  if (endTs * 1000 < now) return "ended";
  if (startTs && startTs * 1000 > now) return "notStarted";
  const diffMs = endTs * 1000 - now;
  const days = Math.floor(diffMs / 86_400_000);
  const hours = Math.floor((diffMs % 86_400_000) / 3_600_000);
  if (days > 0) return `${days}d ${hours}h`;
  const minutes = Math.floor((diffMs % 3_600_000) / 60_000);
  return `${hours}h ${minutes}m`;
}

export function ProjectInfoPanel() {
  const { t } = useTranslation();
  const [engagement, setEngagement] = useState<Engagement | null>(null);
  const [loading, setLoading] = useState(true);
  const [editing, setEditing] = useState(false);
  const [form, setForm] = useState({
    hvvName: "",
    teamMembersText: "",
    startAt: "",
    endAt: "",
    notes: "",
  });
  const [saveStatus, setSaveStatus] = useState<"idle" | "saving" | "saved" | "error">("idle");
  const [now, setNow] = useState(() => Date.now());

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const projectPath = getProjectPath();
      if (!projectPath) {
        setEngagement(null);
        return;
      }
      const row = await engagements.getEngagement(projectPath);
      setEngagement(row);
      if (row) {
        setForm({
          hvvName: row.hvv_name,
          teamMembersText: row.team_members.join(", "),
          startAt: unixToInputValue(row.start_at),
          endAt: unixToInputValue(row.end_at),
          notes: row.notes,
        });
      }
    } catch (e) {
      console.error(t("engagements.loadFailed"), e);
    } finally {
      setLoading(false);
    }
  }, [t]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(id);
  }, []);

  const handleSave = useCallback(async () => {
    const projectPath = getProjectPath();
    if (!projectPath) return;
    setSaveStatus("saving");
    try {
      const teamMembers = form.teamMembersText
        .split(",")
        .map((s) => s.trim())
        .filter((s) => s.length > 0);
      const saved = await engagements.saveEngagement({
        projectPath,
        hvvName: form.hvvName.trim(),
        teamMembers,
        startAt: form.startAt ? inputToIso(form.startAt) : undefined,
        endAt: form.endAt ? inputToIso(form.endAt) : undefined,
        notes: form.notes,
      });
      setEngagement(saved);
      setSaveStatus("saved");
      setTimeout(() => setSaveStatus("idle"), 1500);
      setEditing(false);
    } catch (e) {
      console.error(e);
      setSaveStatus("error");
    }
  }, [form]);

  const hasData =
    engagement &&
    (engagement.hvv_name || engagement.team_members.length > 0 || engagement.start_at || engagement.end_at);

  if (loading) {
    return (
      <div className="px-4 py-2 text-[11px] text-muted-foreground/60 border-b border-border/30">
        {t("engagements.loading")}
      </div>
    );
  }

  if (editing) {
    return (
      <div className="px-4 py-3 border-b border-border/30 bg-muted/10 space-y-2">
        <div className="flex items-center justify-between">
          <span className="text-xs font-medium text-foreground">{t("engagements.title")}</span>
          <button
            type="button"
            className="p-1 rounded text-muted-foreground hover:text-foreground hover:bg-muted/50"
            onClick={() => setEditing(false)}
          >
            <X className="w-3.5 h-3.5" />
          </button>
        </div>
        <input
          className="w-full text-xs bg-background border border-border/50 rounded px-2 py-1.5 outline-none focus:border-accent"
          placeholder={t("engagements.hvvNamePlaceholder")}
          value={form.hvvName}
          onChange={(e) => setForm((f) => ({ ...f, hvvName: e.target.value }))}
        />
        <div className="flex items-center gap-2">
          <label className="text-[10px] text-muted-foreground w-16 text-right">
            {t("engagements.startAt")}
          </label>
          <input
            type="datetime-local"
            className="flex-1 text-xs bg-background border border-border/50 rounded px-2 py-1.5 outline-none focus:border-accent"
            value={form.startAt}
            onChange={(e) => setForm((f) => ({ ...f, startAt: e.target.value }))}
          />
          <label className="text-[10px] text-muted-foreground w-12 text-right">
            {t("engagements.endAt")}
          </label>
          <input
            type="datetime-local"
            className="flex-1 text-xs bg-background border border-border/50 rounded px-2 py-1.5 outline-none focus:border-accent"
            value={form.endAt}
            onChange={(e) => setForm((f) => ({ ...f, endAt: e.target.value }))}
          />
        </div>
        <input
          className="w-full text-xs bg-background border border-border/50 rounded px-2 py-1.5 outline-none focus:border-accent"
          placeholder={t("engagements.teamMembersPlaceholder")}
          value={form.teamMembersText}
          onChange={(e) => setForm((f) => ({ ...f, teamMembersText: e.target.value }))}
        />
        <input
          className="w-full text-xs bg-background border border-border/50 rounded px-2 py-1.5 outline-none focus:border-accent"
          placeholder={t("engagements.notes")}
          value={form.notes}
          onChange={(e) => setForm((f) => ({ ...f, notes: e.target.value }))}
        />
        <div className="flex justify-end gap-2">
          {saveStatus === "saved" && (
            <span className="text-[11px] text-green-400 self-center">{t("engagements.saved")}</span>
          )}
          {saveStatus === "error" && (
            <span className="text-[11px] text-red-400 self-center">{t("engagements.saveFailed")}</span>
          )}
          <button
            type="button"
            className={cn(
              "px-3 py-1 text-xs rounded bg-accent text-accent-foreground hover:bg-accent/90 flex items-center gap-1",
              saveStatus === "saving" && "opacity-50"
            )}
            disabled={saveStatus === "saving"}
            onClick={handleSave}
          >
            <Save className="w-3 h-3" />
            {t("engagements.save")}
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="px-4 py-2 border-b border-border/30 flex items-center gap-3 text-[11px]">
      {!hasData ? (
        <span className="text-muted-foreground/70 flex-1">{t("engagements.empty")}</span>
      ) : (
        <>
          {engagement.hvv_name && (
            <span className="font-medium text-foreground flex items-center gap-1">
              <ChevronDown className="w-3 h-3 text-accent/70" />
              {engagement.hvv_name}
            </span>
          )}
          {(engagement.start_at || engagement.end_at) && (
            <span className="flex items-center gap-1 text-muted-foreground tabular-nums">
              <Calendar className="w-3 h-3 text-blue-400/70" />
              {engagement.start_at
                ? new Date(engagement.start_at * 1000).toLocaleString(undefined, {
                    month: "2-digit",
                    day: "2-digit",
                    hour: "2-digit",
                    minute: "2-digit",
                  })
                : "?"}
              {" — "}
              {engagement.end_at
                ? new Date(engagement.end_at * 1000).toLocaleString(undefined, {
                    month: "2-digit",
                    day: "2-digit",
                    hour: "2-digit",
                    minute: "2-digit",
                  })
                : "?"}
              {engagement.end_at && (
                <span
                  className={cn(
                    "ml-1 px-1.5 py-0.5 rounded",
                    engagement.end_at * 1000 < now
                      ? "bg-red-500/10 text-red-400"
                      : "bg-green-500/10 text-green-400"
                  )}
                >
                  {(() => {
                    const r = formatRemaining(now, engagement.start_at, engagement.end_at);
                    if (r === "ended") return t("engagements.ended");
                    if (r === "notStarted") return t("engagements.notStarted");
                    return t("engagements.remaining", { time: r });
                  })()}
                </span>
              )}
            </span>
          )}
          {engagement.team_members.length > 0 && (
            <span className="flex items-center gap-1 text-muted-foreground">
              <Users className="w-3 h-3 text-purple-400/70" />
              {engagement.team_members.slice(0, 3).join(" · ")}
              {engagement.team_members.length > 3 && (
                <span className="text-muted-foreground/60">
                  +{engagement.team_members.length - 3}
                </span>
              )}
            </span>
          )}
        </>
      )}
      <button
        type="button"
        className="ml-auto p-1 rounded text-muted-foreground hover:text-foreground hover:bg-muted/50"
        onClick={() => setEditing(true)}
        title={t("engagements.edit")}
      >
        <Pencil className="w-3 h-3" />
      </button>
    </div>
  );
}

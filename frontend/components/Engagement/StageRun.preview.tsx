/**
 * Dev-only preview for the stage-run frontend (?preview=intel, DEV only).
 *
 * Composes the REAL components — {@link StageRunView} (left detail pane) +
 * {@link StageRunCard} (chat) — with mock 平安系 data driving the `target_intel`
 * stage, in the app theme via `just dev-fe`. Demonstrates the chat-card →
 * left-detail layout before backend wiring. `&shot=1` freezes a hero frame.
 *
 * NOT a product render path.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { StageRunCard } from "./StageRunCard";
import {
  type StageRunRow,
  type StageRunSummary,
  StageRunView,
  type TechniqueState,
} from "./StageRunView";

const FROZEN =
  new URLSearchParams(typeof window === "undefined" ? "" : window.location.search).get("shot") ===
  "1";

/** target_intel stage config (would come from the stage JSON `coverage_axis`). */
const INTEL_AXIS = ["DNS", "WHOIS", "ASN", "CT", "SUBDOMAIN", "OSINT"];
const INTEL_ROLE = "Recon";

interface OrgSeed {
  id: string;
  name: string;
  ownershipPercent: number | null;
}

const ORGS: OrgSeed[] = [
  { id: "root", name: "中国平安保险（集团）股份有限公司", ownershipPercent: null },
  { id: "bank", name: "平安银行股份有限公司", ownershipPercent: 58 },
  { id: "life", name: "中国平安人寿保险股份有限公司", ownershipPercent: 99 },
  { id: "pc", name: "中国平安财产保险股份有限公司", ownershipPercent: 99 },
  { id: "sec", name: "平安证券股份有限公司", ownershipPercent: 96 },
  { id: "trust", name: "平安信托有限责任公司", ownershipPercent: 99 },
  { id: "fund", name: "平安基金管理有限公司", ownershipPercent: 68 },
  { id: "health", name: "平安健康保险股份有限公司", ownershipPercent: 95 },
  { id: "pension", name: "平安养老保险股份有限公司", ownershipPercent: 99 },
  { id: "lufax", name: "陆金所控股有限公司", ownershipPercent: 41 },
  { id: "tech", name: "平安科技（深圳）有限公司", ownershipPercent: 100 },
];

const RECON_ACTIVITIES = [
  "recon_enrich_assets · 0.zone",
  "subfinder -all -recursive · pingan.com.cn",
  "recon_enrich_assets · quake",
  "gau · waybackurls",
  "amass enum -passive",
  "whois · ICP 备案查询",
];

function cov(...states: TechniqueState[]): Record<string, TechniqueState> {
  const out: Record<string, TechniqueState> = {};
  INTEL_AXIS.forEach((t, i) => {
    out[t] = states[i] ?? "pending";
  });
  return out;
}

function hash(id: string): number {
  let h = 0;
  for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) >>> 0;
  return h;
}

const FROZEN_ROWS: StageRunRow[] = [
  {
    id: "root",
    name: ORGS[0].name,
    ownershipPercent: null,
    status: "passed",
    evidenceCount: 18,
    coverage: cov("found", "found", "found", "found", "found", "found"),
    expanded: true,
    toolLines: [
      { name: "recon_enrich_assets", detail: "0.zone / quake / ENScan ✓", done: true },
      { name: "subfinder -all", detail: "pingan.com.cn · 142 子域", done: true },
      { name: "gau / waybackurls", detail: "历史 URL ✓", done: true },
      { name: "submit_stage_deliverable", detail: "coverage 6/6 · gate PASS", done: true },
    ],
  },
  {
    id: "bank",
    name: ORGS[1].name,
    ownershipPercent: 58,
    status: "running",
    activity: "subfinder -all -recursive · pingan.com.cn",
    evidenceCount: 9,
    coverage: cov("found", "found", "found", "checked_empty", "pending", "pending"),
    expanded: false,
    toolLines: [],
  },
  {
    id: "life",
    name: ORGS[2].name,
    ownershipPercent: 99,
    status: "running",
    activity: "recon_enrich_assets · quake",
    evidenceCount: 6,
    coverage: cov("found", "found", "pending", "pending", "pending", "pending"),
    expanded: false,
    toolLines: [],
  },
  {
    id: "pc",
    name: ORGS[3].name,
    ownershipPercent: 99,
    status: "running",
    activity: "gau · waybackurls",
    evidenceCount: 4,
    coverage: cov("found", "pending", "pending", "pending", "pending", "pending"),
    expanded: false,
    toolLines: [],
  },
  {
    id: "sec",
    name: ORGS[4].name,
    ownershipPercent: 96,
    status: "queued",
    evidenceCount: 0,
    coverage: cov(),
    expanded: false,
    toolLines: [],
  },
  {
    id: "trust",
    name: ORGS[5].name,
    ownershipPercent: 99,
    status: "blocked",
    evidenceCount: 3,
    coverage: cov("found", "checked_empty", "blocked", "pending", "pending", "pending"),
    expanded: false,
    toolLines: [],
  },
  ...ORGS.slice(6).map<StageRunRow>((o) => ({
    id: o.id,
    name: o.name,
    ownershipPercent: o.ownershipPercent,
    status: "pending",
    evidenceCount: 0,
    coverage: cov(),
    expanded: false,
    toolLines: [],
  })),
];

function initialRows(): StageRunRow[] {
  return ORGS.map<StageRunRow>((o, i) => ({
    id: o.id,
    name: o.name,
    ownershipPercent: o.ownershipPercent,
    status: i === 0 ? "passed" : i <= 3 ? "running" : i <= 5 ? "queued" : "pending",
    activity: i >= 1 && i <= 3 ? RECON_ACTIVITIES[hash(o.id) % RECON_ACTIVITIES.length] : undefined,
    evidenceCount: i === 0 ? 18 : i <= 3 ? 3 : 0,
    coverage: i === 0 ? cov("found", "found", "found", "found", "found", "found") : cov("found"),
    expanded: i === 0,
    toolLines:
      i === 0
        ? [
            { name: "recon_enrich_assets", detail: "0.zone / quake / ENScan ✓", done: true },
            { name: "subfinder -all", detail: "pingan.com.cn · 142 子域", done: true },
            { name: "submit_stage_deliverable", detail: "coverage 6/6 · gate PASS", done: true },
          ]
        : [],
  }));
}

function summarize(rows: StageRunRow[]): StageRunSummary {
  const s: StageRunSummary = { total: rows.length, covered: 0, active: 0, queued: 0, blocked: 0 };
  for (const r of rows) {
    if (r.status === "passed") s.covered += 1;
    else if (r.status === "running") s.active += 1;
    else if (r.status === "queued") s.queued += 1;
    else if (r.status === "blocked") s.blocked += 1;
  }
  return s;
}

function Bubble({ side, children }: { side: "user" | "ai"; children: React.ReactNode }) {
  if (side === "user") {
    return (
      <div className="flex justify-end px-3 py-1.5">
        <div className="max-w-[85%] rounded-xl rounded-tr-sm bg-muted/60 px-3 py-2 text-[12.5px]">
          {children}
        </div>
      </div>
    );
  }
  return (
    <div className="flex items-start gap-2 px-3 py-1.5">
      <div className="mt-0.5 grid h-5 w-5 shrink-0 place-items-center rounded-full bg-primary/20 text-[10px] text-primary">
        ✦
      </div>
      <div className="max-w-[85%] rounded-xl rounded-tl-sm border border-border/40 bg-background/40 px-3 py-2 text-[12.5px]">
        {children}
      </div>
    </div>
  );
}

export function StageRunPreviewView() {
  const [rows, setRows] = useState<StageRunRow[]>(FROZEN ? FROZEN_ROWS : initialRows());
  const [k] = useState(3);
  const [open, setOpen] = useState(true);
  const timer = useRef<ReturnType<typeof setInterval> | null>(null);

  const summary = useMemo(() => summarize(rows), [rows]);

  const tick = useCallback(() => {
    setRows((prev) => {
      const next = prev.map((r) => ({ ...r, coverage: { ...r.coverage } }));
      for (const r of next) {
        if (r.status !== "running") continue;
        r.evidenceCount += 1;
        const nextTech = INTEL_AXIS.find((t) => r.coverage[t] === "pending");
        if (nextTech) {
          r.coverage[nextTech] = Math.random() < 0.2 ? "checked_empty" : "found";
          r.activity = RECON_ACTIVITIES[hash(r.id + r.evidenceCount) % RECON_ACTIVITIES.length];
        } else {
          r.status = "passed";
          r.activity = undefined;
        }
      }
      const active = next.filter((r) => r.status === "running").length;
      if (active < k) {
        const q = next.find((r) => r.status === "queued");
        if (q) {
          q.status = "running";
          q.coverage.DNS = "found";
          q.evidenceCount = 1;
          q.activity = RECON_ACTIVITIES[hash(q.id) % RECON_ACTIVITIES.length];
        }
      }
      if (next.filter((r) => r.status === "queued").length < 2) {
        const p = next.find((r) => r.status === "pending");
        if (p) p.status = "queued";
      }
      const done = next.every((r) => r.status === "passed" || r.status === "blocked");
      if (done && timer.current) {
        clearInterval(timer.current);
        timer.current = null;
      }
      return next;
    });
  }, [k]);

  useEffect(() => {
    if (FROZEN) return;
    timer.current = setInterval(tick, 1400);
    return () => {
      if (timer.current) clearInterval(timer.current);
    };
  }, [tick]);

  const onToggleRow = useCallback((id: string) => {
    setRows((prev) => prev.map((r) => (r.id === id ? { ...r, expanded: !r.expanded } : r)));
  }, []);

  return (
    <div className="flex h-screen w-screen items-center justify-center bg-[#0d0d0f] p-5 text-foreground">
      <div className="flex h-[860px] max-h-full w-[1140px] max-w-full flex-col overflow-hidden rounded-xl border border-border/60 bg-background shadow-2xl">
        <div className="flex items-center gap-2 border-b border-border/60 px-3 py-2">
          <div className="flex items-center gap-2 rounded-lg border border-border/60 bg-muted/40 px-3 py-1.5 text-[13px]">
            搞一下平安 <span className="text-muted-foreground/60">✕</span>
          </div>
          <span className="flex-1" />
          <span className="text-muted-foreground/50">＋</span>
        </div>

        <div className="flex min-h-0 flex-1">
          {/* LEFT: detail pane — StageRunView when the chat card is opened */}
          <div className="flex min-w-0 flex-[1.4] flex-col border-r border-border/60">
            <div className="flex items-center gap-2 border-b border-border/60 bg-background/60 px-3 py-2 text-[12px]">
              <span className="text-muted-foreground/70">◀ 返回</span>
              <span className="font-semibold">详情 · Target Intel 流水线</span>
            </div>
            <div className="min-h-0 flex-1 overflow-y-auto py-2">
              {open ? (
                <StageRunView
                  rows={rows}
                  summary={summary}
                  concurrency={k}
                  stageLabel="Target Intel"
                  stageTag="被动 · zero-touch"
                  roleLabel={INTEL_ROLE}
                  coverageAxis={INTEL_AXIS}
                  onToggleRow={onToggleRow}
                />
              ) : (
                <div className="flex h-full items-center justify-center px-6 text-center text-[12px] text-muted-foreground/50">
                  点右边聊天里的「Target Intel」卡片 → 这里显示 11 家并行收集的同步视图
                </div>
              )}
            </div>
          </div>

          {/* RIGHT: chat pane — clean, only a compact card */}
          <div className="flex w-[400px] shrink-0 flex-col">
            <div className="flex items-center gap-2 border-b border-border/60 bg-background/60 px-3 py-2 text-[12px]">
              <span className="h-3 w-3 shrink-0 rounded-full border-2 border-emerald-500/60" />
              <span className="font-semibold">Target Intel</span>
              <span className="text-muted-foreground tabular-nums">2/12</span>
              <span className="text-muted-foreground/60"> · 被动情报</span>
            </div>
            <div className="min-h-0 flex-1 overflow-y-auto py-2">
              <Bubble side="user">搞一下平安集团</Bubble>
              <Bubble side="ai">
                范围已锁定 <b>11 家</b> ✅ 过 scoping gate，进入 <b>Target Intel</b>。我已并行起 {k}{" "}
                个 Recon 收集者，每家各跑各的、各过各的 gate：
              </Bubble>

              <StageRunCard
                stageLabel="Target Intel"
                roleTag="Recon · 被动"
                summary={summary}
                open={open}
                onOpen={() => setOpen((v) => !v)}
              />

              <Bubble side="ai">
                {summary.covered}/{summary.total} 家已覆盖
                {summary.blocked > 0 ? `，${summary.blocked} 家 BLOCK 待复核` : ""}
                。点上面卡片在左侧看实时进度；全过 gate 后自动进 EAS（主动探测）。
              </Bubble>
            </div>
            <div className="border-t border-border/60 p-3">
              <div className="rounded-xl border border-border/60 bg-background px-3 py-2.5">
                <div className="text-[12.5px] text-muted-foreground/50">输入消息…</div>
                <div className="mt-2 flex items-center gap-3 text-muted-foreground/50">
                  <span>⚙︎ Red Team</span>
                  <span className="flex-1" />
                  <span>➤</span>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

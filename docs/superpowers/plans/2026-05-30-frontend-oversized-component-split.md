# 前端超预算组件拆分 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现此计划。
> 配套：`docs/superpowers/plans/2026-05-30-arch-health-backlog.md`（本计划是其 **P2 前端** 部分的详细展开）。
> 作者：MCP-5（ui_designer）。分工：MCP-3 后端文件拆分 / MCP-4 类型收敛(I5) / 本计划 前端组件拆分。

**目标：** 把超过 800 行 TS/TSX 预算（`scripts/check_file_sizes.sh`）的前端文件按职责拆成小而专注的模块，**行为零变更**，让 `arch-check.yml` 的 file-size gate 对前端转绿。
**架构：** 纯结构性重构——把单文件内已经存在的子组件 / 纯函数 / 类型原样搬到 sibling 模块，主文件只留「壳」（状态 + 布局 + 组装）。公共导入路径保持不变，外部 import 不受影响。对搬出的纯函数补单测锁定行为。
**技术栈：** React 19 + TypeScript 6 + Vite 8 + Vitest + Biome。

---

## 背景与范围

`bash scripts/check_file_sizes.sh`（2026-05-30 实跑）前端当前超预算（已排除 `mocks/`）：

| 行数 | 文件 | 归属 | 本计划 |
|---|---|---|---|
| 818 | `frontend/components/TargetPanel/TargetSurfaceWorkbench.tsx` | UI | **Task 1–6 主线** |
| 814 | `frontend/components/ToolManager/hooks/useToolInstall.ts` | hook | 附录 A（同模式，独立 commit） |

近预算（未超但 >700，预防性，按附录 B 优先级排）：`Settings/ProviderSettings/index.tsx` 794 · `VulnIntelPanel/PocTab.tsx` 757 · `PipelinePanel/PipelinePanel.tsx` 746 · `TargetPanel/TargetGroupedView.tsx` 743 · `PipelinePanel/DagComponents.tsx` 739 · `lib/conversation-db-sync.ts` 707。

> **为什么先拆 `TargetSurfaceWorkbench.tsx`**：它是当前唯一**真正越过 800 红线**的 UI 组件文件，且内部已天然分成 1 个壳 + 6 个 tab + 5 个展示原语 + 5 个纯函数，拆分缝隙清晰、风险最低。

**外部引用面（已核对）**：仅 `frontend/components/TargetPanel/TargetGroupedView.tsx` 通过 `./TargetSurfaceWorkbench` 引入 `TargetSurfaceWorkbench`。因此 `TargetSurfaceWorkbench.tsx` 路径与具名导出 **必须保持不变**；新增内部文件放进 `TargetPanel/surface/` 子目录。

---

## 目标文件结构

```text
frontend/components/TargetPanel/
  TargetSurfaceWorkbench.tsx        # 壳：props + tab state + useMemo 派生 + 布局 + tab 分发（目标 ≤140 行）
  surface/
    types.ts                        # SurfaceTab, SURFACE_TABS, SitemapItem, SensitiveFinding（本文件内现有局部类型）
    surfaceModel.ts                 # 纯函数：isHttpPort / buildSitemapItems / buildSensitiveFindings / formatLatestEvidence / formatTime
    surfaceModel.test.ts            # 新增：纯函数单测（锁定行为）
    SurfaceParts.tsx                # 展示原语：StageButton / Section / Kv / Metric / EmptyInline / EmptyPanel
    tabs/
      IdentityTab.tsx               # 现 228–263
      SurfaceTabView.tsx            # 现 265–392
      SitemapTab.tsx                # 现 394–433
      JsApiTab.tsx                  # 现 435–499
      SensitiveTab.tsx              # 现 501–550
      EvidenceTab.tsx               # 现 552–634
```

职责边界：
- **壳** 只持有 `activeTab` 状态、调用 `useTargetSurfaceData`、用 `useMemo` 派生（httpPorts / sitemapItems / sensitiveFindings / counts / lastEvidenceLabel）、渲染 stage 头 + tab 条 + `switch(activeTab)` 分发。
- **tabs/** 每个 tab 是纯展示组件，props 即现有 inline 组件签名（见下方各 Task，签名原样保留）。
- **SurfaceParts.tsx** 收纳无业务、可跨 tab 复用的展示原语。
- **surfaceModel.ts** 收纳零 React 依赖的纯函数，便于单测。

---

## Task 0 — 基线与定位

**文件：** 无（只读）
**步骤：**
1. 运行基线，记录当前行数与 gate 状态：
```bash
wc -l frontend/components/TargetPanel/TargetSurfaceWorkbench.tsx
bash scripts/check_file_sizes.sh; echo "exit=$?"
```
2. 确认外部引用面未变（应只有 TargetGroupedView）：
```bash
rg -rn "TargetSurfaceWorkbench" frontend --glob '*.ts' --glob '*.tsx' | rg -v "surface/"
```
**验证：** gate 输出含 `TargetSurfaceWorkbench.tsx: 818 lines > 800`；引用面只有 1 处。
**提交：** 无（基线步骤）。

---

## Task 1 — 抽出共享类型 `surface/types.ts`

**文件：**
- 新建 `frontend/components/TargetPanel/surface/types.ts`
- 改 `TargetSurfaceWorkbench.tsx`

**步骤：**
1. 新建 `surface/types.ts`，把主文件中现有的局部类型与常量原样搬入（`SurfaceTab`、`SURFACE_TABS`，以及文件内现有的 `SitemapItem`、`SensitiveFinding` 接口声明）：
```ts
export type SurfaceTab =
  | "identity"
  | "surface"
  | "sitemap"
  | "js-api"
  | "sensitive"
  | "evidence";

export const SURFACE_TABS: Array<{ id: SurfaceTab; label: string }> = [
  { id: "identity", label: "Identity" },
  { id: "surface", label: "Surface" },
  { id: "sitemap", label: "Sitemap" },
  { id: "js-api", label: "JS / API" },
  { id: "sensitive", label: "Sensitive" },
  { id: "evidence", label: "Evidence" },
];

// 把 TargetSurfaceWorkbench.tsx 中现有的 SitemapItem / SensitiveFinding
// interface 原样剪切到此处并 export（字段不改）。
```
2. 在 `TargetSurfaceWorkbench.tsx` 删除这些本地声明，改为 `import { SURFACE_TABS, type SurfaceTab, type SitemapItem, type SensitiveFinding } from "./surface/types";`

**验证：**
```bash
pnpm typecheck
```
预期：通过（仅移动类型，无逻辑变化）。
**提交：** `refactor(target-surface): extract shared types to surface/types.ts`

---

## Task 2 — 抽出纯函数 `surface/surfaceModel.ts` + 单测

**文件：**
- 新建 `frontend/components/TargetPanel/surface/surfaceModel.ts`
- 新建 `frontend/components/TargetPanel/surface/surfaceModel.test.ts`
- 改 `TargetSurfaceWorkbench.tsx`

**步骤：**
1. 新建 `surfaceModel.ts`，把以下纯函数**原样**搬入并 export（函数体逐字复制，签名不变）：
```ts
import type { DirectoryEntry } from "@/lib/pentest/api";
import type { PortInfo } from "@/lib/pentest/types";
import type {
  JsAnalysisResult,
  PassiveScanLog,
  TargetAsset,
} from "@/lib/security-analysis";
import type { SensitiveFinding, SitemapItem } from "./types";

export function isHttpPort(port: PortInfo): boolean { /* 现 718–728 原样 */ }

export function buildSitemapItems(
  assets: TargetAsset[],
  directoryEntries: DirectoryEntry[]
): SitemapItem[] { /* 现 730–766 原样 */ }

export function buildSensitiveFindings(
  jsResults: JsAnalysisResult[],
  passiveScans: PassiveScanLog[]
): SensitiveFinding[] { /* 现 768–802 原样 */ }

export function formatLatestEvidence(
  timelineCreatedAt?: string,
  logCreatedAt?: number
): string | null { /* 现 804–808 原样 */ }

export function formatTime(value: string | number): string { /* 现 810–818 原样 */ }
```
2. 主文件删除这些函数，改为从 `./surface/surfaceModel` import 已用到的（`isHttpPort`、`buildSitemapItems`、`buildSensitiveFindings`、`formatLatestEvidence`）。
3. 新建 `surfaceModel.test.ts`，对纯函数补行为锁定测试：
```ts
import { describe, expect, it } from "vitest";
import { formatTime, isHttpPort } from "./surfaceModel";

describe("formatTime", () => {
  it("echoes raw value on unparseable input", () => {
    expect(formatTime("not-a-date")).toBe("not-a-date");
  });
  it("renders HH:MM:SS for a valid timestamp", () => {
    // 固定时区无关：仅断言形如 2 位:2 位:2 位
    expect(formatTime("2026-01-01T13:05:09Z")).toMatch(/^\d{2}:\d{2}:\d{2}$/);
  });
});

describe("isHttpPort", () => {
  it("classifies common http/https ports", () => {
    // 用最小 PortInfo 夹具；字段按 @/lib/pentest/types 的 PortInfo 形状构造
    expect(isHttpPort({ port: 80 } as never)).toBe(true);
  });
});
```
> 注：`buildSitemapItems` / `buildSensitiveFindings` 的夹具较重，若构造成本高，本 Task 至少覆盖 `formatTime` + `isHttpPort`；其余两个在数据形状稳定后补（记入附录 C）。

**验证：**
```bash
pnpm exec vitest run frontend/components/TargetPanel/surface/surfaceModel.test.ts
pnpm typecheck
```
预期：测试通过、typecheck 通过。
**提交：** `refactor(target-surface): extract pure helpers + unit tests`

---

## Task 3 — 抽出展示原语 `surface/SurfaceParts.tsx`

**文件：**
- 新建 `frontend/components/TargetPanel/surface/SurfaceParts.tsx`
- 改 `TargetSurfaceWorkbench.tsx`

**步骤：**
1. 新建 `SurfaceParts.tsx`，搬入并 export（函数体原样）：`StageButton`（现 203–226）、`Section`（636–655）、`Kv`（656–665）、`Metric`（667–677）、`EmptyInline`（679–692）、`EmptyPanel`（694–716）。保留各自现有签名：
```tsx
export function StageButton({ icon, label, muted = false }: {
  icon: React.ReactNode; label: string; muted?: boolean;
}) { /* 原样 */ }

export function Section({ title, subtitle, children }: {
  title: string; subtitle?: string; children: React.ReactNode;
}) { /* 原样 */ }

export function Kv({ label, value, mono = false }: {
  label: string; value: string; mono?: boolean;
}) { /* 原样 */ }

export function Metric({ icon, label, value }: {
  icon: React.ReactNode; label: string; value: number;
}) { /* 原样 */ }

export function EmptyInline({ label, loading }: { label: string; loading?: boolean }) { /* 原样 */ }

export function EmptyPanel({ icon, title, body, loading }: {
  icon: React.ReactNode; title: string; body: string; loading?: boolean;
}) { /* 原样 */ }
```
2. 主文件删除这些声明（壳里若直接用到 `StageButton` 则保留 import）。

**验证：** `pnpm typecheck`（此时 tab 组件仍在主文件，会从主文件内引用这些原语 → 临时 import 自 `./surface/SurfaceParts`；Task 4 搬 tab 时一并带走）。
**提交：** `refactor(target-surface): extract presentational primitives`

---

## Task 4 — 抽出 6 个 tab 到 `surface/tabs/*`

**文件：**
- 新建 `surface/tabs/IdentityTab.tsx` `SurfaceTabView.tsx` `SitemapTab.tsx` `JsApiTab.tsx` `SensitiveTab.tsx` `EvidenceTab.tsx`
- 改 `TargetSurfaceWorkbench.tsx`

**步骤：** 对每个 tab，新建文件、`export function <Tab>(...)` 函数体原样搬入、补齐 import（`react` 图标、`@/lib/*` 类型、`../SurfaceParts`、`../surfaceModel`、`../types`）。签名保持不变：

```tsx
// surface/tabs/SurfaceTabView.tsx
export function SurfaceTabView({
  target, httpPorts, endpointCount, jsCount, fingerprints, loading,
}: {
  target: Target; httpPorts: PortInfo[]; endpointCount: number;
  jsCount: number; fingerprints: Fingerprint[]; loading: boolean;
}) { /* 现 265–392 原样 */ }
```
```tsx
// surface/tabs/JsApiTab.tsx
export function JsApiTab({ endpoints, jsResults, loading }: {
  endpoints: ApiEndpoint[]; jsResults: JsAnalysisResult[]; loading: boolean;
}) { /* 现 435–499 原样 */ }
```
```tsx
// surface/tabs/SensitiveTab.tsx
export function SensitiveTab({ findings, sensitiveCount, loading }: {
  findings: SensitiveFinding[]; sensitiveCount: number; loading: boolean;
}) { /* 现 501–550 原样 */ }
```
```tsx
// surface/tabs/EvidenceTab.tsx
export function EvidenceTab({ target, timeline, logs, loading }: {
  target: Target; timeline: TimelineEntry[];
  logs: Array<{ id: number; action: string; status: string; toolName: string | null; createdAt: number }>;
  loading: boolean;
}) { /* 现 552–634 原样；formatLatestEvidence/formatTime 改 import from ../surfaceModel */ }
```
`IdentityTab`（228–263）、`SitemapTab`（394–433）同法搬出，签名见原文件。

**验证：** `pnpm typecheck`
**提交：** `refactor(target-surface): move tab views into surface/tabs`

---

## Task 5 — 收口主文件为「壳」

**文件：** `TargetSurfaceWorkbench.tsx`

**步骤：**
1. 主文件现在应只剩：imports + `export function TargetSurfaceWorkbench`（壳）。补齐对各 tab / 原语 / model / types 的 import：
```tsx
import { SURFACE_TABS, type SurfaceTab } from "./surface/types";
import { StageButton } from "./surface/SurfaceParts";
import { buildSensitiveFindings, buildSitemapItems, formatLatestEvidence, isHttpPort } from "./surface/surfaceModel";
import { IdentityTab } from "./surface/tabs/IdentityTab";
import { SurfaceTabView } from "./surface/tabs/SurfaceTabView";
import { SitemapTab } from "./surface/tabs/SitemapTab";
import { JsApiTab } from "./surface/tabs/JsApiTab";
import { SensitiveTab } from "./surface/tabs/SensitiveTab";
import { EvidenceTab } from "./surface/tabs/EvidenceTab";
```
2. `pnpm exec biome check --write` 整理 import 顺序（biome organizeImports）。

**验证：**
```bash
wc -l frontend/components/TargetPanel/TargetSurfaceWorkbench.tsx   # 预期 ≤140
bash scripts/check_file_sizes.sh; echo "exit=$?"                   # TargetSurfaceWorkbench 不再出现
```
**提交：** `refactor(target-surface): slim workbench to shell`

---

## Task 6 — 全量验证

**文件：** 无
**步骤/验证：**
```bash
pnpm typecheck
pnpm exec biome check frontend/components/TargetPanel/surface frontend/components/TargetPanel/TargetSurfaceWorkbench.tsx
pnpm exec vitest run frontend/components/TargetPanel
bash scripts/check_file_sizes.sh
```
预期：typecheck 通过；biome `No fixes applied`；TargetPanel 现有测试（含 `topology/buildTopologyModel.test.ts`）全过 + 新增 surfaceModel 测试通过；file-size gate 不再列 `TargetSurfaceWorkbench.tsx`。
**提交：** 若前序每步已 commit，则本步无新改动。

---

## 附录 A — `useToolInstall.ts`（814 行，同模式独立处理）

hook 文件，拆分思路：把安装流程的纯状态机 / 校验 / 平台分支抽到 `ToolManager/hooks/toolInstall/`（如 `reducer.ts` / `steps.ts` / `platform.ts`），`useToolInstall.ts` 只保留 hook 编排。**独立 commit**，验证同 Task 6（typecheck + vitest + gate）。需先读该文件确认 reducer/effect 边界后再细化（不在本计划主线，避免无证据占位）。

## 附录 B — 近预算 UI 文件优先级（预防性）
1. `Settings/ProviderSettings/index.tsx` 794 → 抽 provider 表单分区子组件
2. `VulnIntelPanel/PocTab.tsx` 757 → 抽 PoC 列表项 / 过滤器
3. `PipelinePanel/PipelinePanel.tsx` 746 + `DagComponents.tsx` 739 → DAG 渲染原语下沉
4. `TargetPanel/TargetGroupedView.tsx` 743 → 分组头 / 行渲染抽子组件
5. `lib/conversation-db-sync.ts` 707 → 按「读/写/diff」三段拆
每个独立 commit、行为零变更、逐文件 `tsc`+`vitest` 验证。

## 附录 C — 顺带可做的去重（非阻塞）
`surfaceModel.formatTime`（`toLocaleTimeString` HH:MM:SS）与 `lib/time.ts` 家族重叠。本轮先保持在 `surfaceModel.ts`；后续可在 `lib/time.ts` 增 `formatClockTime(value)` 统一（与已完成的 formatDuration/formatRelativeAgo 收编一致）。

---

## 自检

1. **范围覆盖度：** 红线文件 `TargetSurfaceWorkbench.tsx` → Task 1–6 全覆盖；第二个红线 `useToolInstall.ts` → 附录 A（独立）。✓
2. **占位符扫描：** 各 Task 给出真实路径、真实签名、真实验证命令；函数体为「原样搬移」（合法的重构指令，非 TODO）。纯函数补了真实测试代码。✓
3. **类型一致性：** `SurfaceTab`/`SURFACE_TABS`/`SitemapItem`/`SensitiveFinding` 在 Task 1 定义，Task 2/4/5 引用名一致；tab 组件签名与原文件逐字一致。✓
4. **行为保持：** 全程「移动 + 重新 import」，无逻辑改写；用 typecheck + 现有 + 新增测试 + file-size gate 四道验证守住。✓

# 前端指紋顯示 實現計畫（問題四）

> **面向 AI 代理的工作者：** 用 `.cursor/skills/executing-plans` 逐任務實現。每個任務單獨 commit。

**目標：** 讓 TargetPanel 的 Burp-style workbench 能看到 `fingerprints` 表的完整指紋（category / name / version / confidence / cpe / evidence），不再只顯示 `network_endpoints` 的基本 service 字串。
**架構：** 純前端。指紋資料早已存在於 `WebOriginVM.fingerprints`（`surfaceHierarchy.ts:630` 已 `attachFingerprintEvidence`），只是新 workbench 的 `WebOriginsTab` 沒渲染。給 `WebOriginsTab` 的 origin 詳情加一個「Fingerprints」子 tab，複用 `SurfaceTabView.tsx:125-158` 既有渲染樣式抽成共用元件。零後端、零 gate、零 schema。
**技術棧：** React 19 + TypeScript + Tailwind（Vitest 測試）。

---

## 背景與根因（實讀證據）

- 後端 `fingerprints` 表齊全：`id/category/name/version/confidence/cpe/evidence/source/detected_at`（`backend/crates/golish-db/src/models/pentest.rs:342`）。
- 前端型別 `Fingerprint` 已存在並被掛到 `WebOriginVM.fingerprints`（`frontend/components/TargetPanel/surface/surfaceHierarchy.ts:145` 宣告、`:630` 填充、`:858` `attachFingerprintEvidence`）。
- **缺口**：`frontend/components/TargetPanel/surface/tabs/WebOriginsTab.tsx` 的 `DETAIL_TABS`（`:13`）沒有 Fingerprints tab，`OriginDetail`（`:366`）不渲染 `origin.fingerprints`。`NetworkEndpointsTab.tsx:64` 只渲染 `endpoint.service`（= `serviceLabel()` 拼接 name+product+version，來自 `network_endpoints`，與 `fingerprints` 表是兩套來源）。
- 舊 `SurfaceTabView.tsx:125-158` 有一段可複用的 Fingerprints Section 渲染。

## 檔案結構

| 檔案 | 動作 | 職責 |
|---|---|---|
| `frontend/components/TargetPanel/surface/tabs/FingerprintList.tsx` | 新建 | 可複用的指紋清單元件（讀 `Fingerprint[]`，渲染 category/name/version/confidence/cpe） |
| `frontend/components/TargetPanel/surface/tabs/WebOriginsTab.tsx` | 改 | `DETAIL_TABS` 加 `fingerprints` tab；`OriginDetail` 渲染 `<FingerprintList>`；概覽表加 Fingerprints 計數欄 |
| `frontend/components/TargetPanel/surface/tabs/SurfaceTabView.tsx` | 改 | 把行內 Fingerprints Section 換成 `<FingerprintList>`（DRY，去重） |
| `frontend/components/TargetPanel/surface/tabs/FingerprintList.test.tsx` | 新建 | 元件單測（有指紋/空/version 缺省/cpe） |

---

## Task 1：抽出共用 `FingerprintList` 元件

**檔案**：`frontend/components/TargetPanel/surface/tabs/FingerprintList.tsx`（新建）

**步驟**：先確認 `Fingerprint` 型別欄位（`surfaceHierarchy.ts` import 來源）——至少有 `id/category/name/version?/confidence/cpe?/evidence`。以此寫元件：

```tsx
import { ScanSearch } from "lucide-react";
import { EmptyInline, Section } from "../SurfaceParts";
import type { Fingerprint } from "../surfaceHierarchy";

export function FingerprintList({
  fingerprints,
  loading = false,
  limit = 50,
}: {
  fingerprints: Fingerprint[];
  loading?: boolean;
  limit?: number;
}) {
  return (
    <Section
      title="Fingerprints"
      subtitle={`${fingerprints.length} detected`}
    >
      {fingerprints.length === 0 ? (
        <EmptyInline loading={loading} label="No service/version fingerprint yet (nmap -sV / whatweb)" />
      ) : (
        <div className="space-y-1.5">
          {fingerprints.slice(0, limit).map((fp) => (
            <div key={fp.id} className="rounded border border-border/20 bg-muted/5 px-2 py-1.5">
              <div className="flex items-center gap-2 text-[11px]">
                <ScanSearch className="h-3.5 w-3.5 flex-shrink-0 text-purple-300/80" />
                <span className="rounded bg-purple-500/10 px-1.5 py-0.5 text-[9px] text-purple-300">
                  {fp.category}
                </span>
                <span className="min-w-0 flex-1 truncate text-foreground/85">{fp.name}</span>
                {fp.version && (
                  <span className="font-mono text-[10px] text-muted-foreground">{fp.version}</span>
                )}
                <span className="text-[9px] text-muted-foreground">
                  {Math.round((fp.confidence ?? 0) * (fp.confidence <= 1 ? 100 : 1))}%
                </span>
              </div>
              {fp.cpe && (
                <p className="mt-1 truncate font-mono text-[9px] text-muted-foreground" title={fp.cpe}>
                  {fp.cpe}
                </p>
              )}
            </div>
          ))}
          {fingerprints.length > limit && (
            <p className="text-[9px] text-muted-foreground">
              +{fingerprints.length - limit} more
            </p>
          )}
        </div>
      )}
    </Section>
  );
}
```

> 若 `Fingerprint` 沒有 `cpe` 欄位，刪掉 cpe 區塊；`confidence` 的百分比換算按實際型別（`SurfaceTabView.tsx:151` 用 `Math.round(fingerprint.confidence)`，若已是 0–100 就直接用，勿再 ×100——**先讀型別再定**）。

**驗證**：
```bash
cd frontend && npx tsc --noEmit
# 预期：无类型错误（Fingerprint import 正确）
```
**Commit**：`feat(frontend): extract reusable FingerprintList component`

---

## Task 2：`WebOriginsTab` 加 Fingerprints tab

**檔案**：`frontend/components/TargetPanel/surface/tabs/WebOriginsTab.tsx`

**步驟 2.1** — import：
```tsx
import { FingerprintList } from "./FingerprintList";
```

**步驟 2.2** — `DETAIL_TABS`（`:13`）與 `OriginDetailTab` 型別加一項：
```tsx
type OriginDetailTab = "overview" | "sitemap" | "apis" | "js" | "params" | "fingerprints" | "evidence";

const DETAIL_TABS: Array<{ id: OriginDetailTab; label: string }> = [
  { id: "overview", label: "Overview" },
  { id: "sitemap", label: "Sitemap" },
  { id: "apis", label: "APIs" },
  { id: "js", label: "JS" },
  { id: "params", label: "Params" },
  { id: "fingerprints", label: "Fingerprints" },
  { id: "evidence", label: "Evidence" },
];
```

**步驟 2.3** — `OriginDetail` 的 tab 渲染區（`:445` 一帶）加：
```tsx
        {activeDetailTab === "params" && <ParamList origin={origin} />}
        {activeDetailTab === "fingerprints" && (
          <FingerprintList fingerprints={origin.fingerprints} />
        )}
        {activeDetailTab === "evidence" && <EvidenceList origin={origin} />}
```

**步驟 2.4**（可選但建議）— origin 概覽表加一欄 Fingerprints 計數，讓使用者一眼看到哪個 origin 有指紋。在 `WebOriginsTab` 的 `<thead>`（`:509` Evidence 欄後）加 `<th>FP</th>`，`<tbody>` 對應加：
```tsx
                      <td className="px-2 py-2">
                        <CountCell value={origin.fingerprints.length} />
                      </td>
```

**驗證**：
```bash
cd frontend && npx tsc --noEmit && just test-fe
# 预期：类型过；现有 WebOriginsTab 相关测试不回归
```
**Commit**：`feat(frontend): show fingerprints in WebOrigins detail tab`

---

## Task 3：`SurfaceTabView` 複用 `FingerprintList`（DRY）

**檔案**：`frontend/components/TargetPanel/surface/tabs/SurfaceTabView.tsx`

**步驟**：把 `:125-158` 的行內 `<Section title="Fingerprints">…</Section>` 整段換成：
```tsx
import { FingerprintList } from "./FingerprintList";
// ...
          <FingerprintList fingerprints={fingerprints} loading={loading} limit={10} />
```
保留上方 Metric 的 `value={fingerprints.length}`（`:121`）不動。

**驗證**：
```bash
cd frontend && npx tsc --noEmit && just check-fe
# 预期：biome + typecheck 全过；无重复实现
```
**Commit**：`refactor(frontend): reuse FingerprintList in SurfaceTabView`

---

## Task 4：元件單測

**檔案**：`frontend/components/TargetPanel/surface/tabs/FingerprintList.test.tsx`（新建）

**步驟**：
```tsx
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { FingerprintList } from "./FingerprintList";

const fp = (over = {}) => ({
  id: "1", category: "server", name: "nginx", version: "1.25.3",
  confidence: 0.9, evidence: [], source: "whatweb", ...over,
});

describe("FingerprintList", () => {
  it("renders detected fingerprints", () => {
    render(<FingerprintList fingerprints={[fp() as any]} />);
    expect(screen.getByText("nginx")).toBeInTheDocument();
    expect(screen.getByText("1.25.3")).toBeInTheDocument();
    expect(screen.getByText("server")).toBeInTheDocument();
  });
  it("shows empty state", () => {
    render(<FingerprintList fingerprints={[]} />);
    expect(screen.getByText(/No service\/version fingerprint/)).toBeInTheDocument();
  });
  it("omits version when absent", () => {
    render(<FingerprintList fingerprints={[fp({ version: undefined }) as any]} />);
    expect(screen.queryByText("1.25.3")).not.toBeInTheDocument();
  });
});
```

**驗證**：
```bash
cd frontend && just test-fe
# 预期：3 个新用例全过
```
**Commit**：`test(frontend): cover FingerprintList component`

---

## 收口

```bash
just check-fe && just test-fe
```
全綠後：更新 `agent-progress.md`（本輪目標/證據）、`feature_list.json`（若列了此功能）、`docs/modules/frontend/components.md`（TargetPanel workbench 卡新增 Fingerprints tab 說明）。

## 自檢

1. **規格覆蓋**：問題四「指紋看不到」→ Task 2 origin 詳情 Fingerprints tab + Task 2.4 概覽計數欄，覆蓋。
2. **占位符掃描**：無 TODO / 待定。cpe 與 confidence 換算已標「先讀型別再定」的判斷點。
3. **型別一致**：`FingerprintList` 的 props 型別 `Fingerprint` 與 `WebOriginVM.fingerprints`（`surfaceHierarchy.ts:145`）一致；`OriginDetailTab` union 三處（型別/DETAIL_TABS/渲染）同步加 `fingerprints`。

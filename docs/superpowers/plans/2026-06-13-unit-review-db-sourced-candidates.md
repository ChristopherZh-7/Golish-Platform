# unit_review 候选改为「DB 取数」实现计划（不依赖 LLM 搬数组）

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现此计划。

**目标：** scoping 的子公司审核表（`ask_human(unit_review)`）不再依赖 LLM 把 10-18 条候选原样塞进 `context`（mimo 文本工具调用会把 JSON 数组搞坏 → 表格空）。改成：agent 只传**小而可靠的 `organization_id`**，审核表**自己从 DB 拉候选**（`organization_candidates_list`）填表。

**架构：** discover（auto_promote=off）已把候选落进 `org.intel.engagement.candidates`（`organization_candidates_list(id)` 可取）。让 `ask_human(unit_review)` 的 `context` 携带 `{"organization_id":"<uuid>"}`（agent 一直能可靠传 org_id）；前端 `AskHumanInline` 检测到 org_id → 调 `organization_candidates_list` → 映射成审核行。同时把 `parseReviewContext` 做成容错（数组 / 字符串化 JSON / {items:[]} / 纯文本逐行）兜底。只动**前端 + 一处 Rust schema 文案 + methodology**，不动 ask_human 事件管线。

**技术栈：** React/TS（AskHumanInline, ScopeReviewTable），Rust（tool schema 文案），harness methodology md，vitest。

---

## 背景与现状（必读）

- `AskHumanState`（`AskHumanInline.tsx:16`）只有 `{requestId, sessionId, question, inputType, options, context}` —— **无 org 字段**；context 是自由字符串，已经能到前端。
- `parseReviewContext`（同文件:53）：空→`[]`；`JSON.parse` 失败→`[]`。`ScopeReviewTable` 拿 `initial`（数组）渲染；非数组/空→一个空行→显示占位符。
- 已有命令：`organization_candidates_list(id)`（`frontend/lib/api/organizations.ts:170`，invoke `organization_candidates_list`）→ 返回 `OrganizationCandidates {organizations: OrganizationCandidate[], targets: []}`。`OrganizationCandidate` 有 `value`（名称）、`evidence`（`raw.scale`/`raw.status`）。
- discover（auto_promote=false）+ `propose_candidates` 都把候选写进 `org.intel.engagement.candidates`，所以 `organization_candidates_list(org_id)` 拿得到子公司候选。
- agent 可靠传 `organization_id`（discover/propose 都对）；不可靠的是大数组。
- `ask_human` schema（`definitions.rs:70`）`context` 文案是泛泛的「Additional context about why you need this」——会误导 LLM 不传结构化数据。

---

## 文件结构

- `frontend/components/AIChatPanel/AskHumanInline.tsx` — `parseReviewContext` 容错 + 解析出 `organization_id`；unit_review 时按 org_id 拉候选。
- `frontend/components/AIChatPanel/ScopeReviewTable.tsx` — 新增 `candidatesToUnitRows` 纯函数（OrganizationCandidate[] → 行）+ 测试。
- `frontend/lib/api/organizations.ts` — 复用现有 `listOrganizationCandidates`（确认导出名）。
- `backend/crates/golish-agent-kit/src/tool_definitions/definitions.rs` — `ask_human` 的 `context` 文案改清楚（unit_review 传 `{"organization_id":"<uuid>"}` 或候选 JSON 数组）。
- `resources/harness/stages/scoping.methodology.md` — step 3 改成 `context={"organization_id": <org id>}`。

---

## Task 1 — ScopeReviewTable: 候选→行 的纯函数 + 测试

**文件：** `frontend/components/AIChatPanel/ScopeReviewTable.tsx`、`ScopeReviewTable.test.tsx`

**步骤：**

1. 在 `ScopeReviewTable.tsx` 导出一个纯函数，把 DB 候选映射成 unit_review 行（名称带投资比标签，便于人判断）：
```ts
import type { OrganizationCandidate } from "@/lib/api/organizations";

/** Map DB engagement candidates (organizations) into unit_review rows. Ownership
 * (from evidence.raw.scale) is appended to the name label so the user can judge
 * at a glance; non-org candidates are ignored. */
export function candidatesToUnitRows(candidates: OrganizationCandidate[]): ScopeReviewRow[] {
  return candidates
    .map((c) => {
      const name = (c.value ?? "").trim();
      if (!name) return null;
      const raw = (c.evidence as { raw?: { scale?: string } } | undefined)?.raw;
      const scale = raw?.scale?.trim();
      return { name: scale ? `${name} (${scale})` : name, aliases: "", domains: "" } as ScopeReviewRow;
    })
    .filter((r): r is ScopeReviewRow => r !== null);
}
```
   （`OrganizationCandidate` 类型从 `@/lib/api/organizations` 导入；若该类型未导出，改从 `@/lib/generated` 或在 organizations.ts 加 `export type`。先确认导出名。）

2. 测试（`ScopeReviewTable.test.tsx`）：
```ts
import { candidatesToUnitRows } from "./ScopeReviewTable";

it("maps org candidates to rows with ownership label", () => {
  const rows = candidatesToUnitRows([
    { id: "", kind: "organization", label: "n", value: "平安银行股份有限公司", source: "rb", confidence: 0.8, status: "", evidence: { raw: { scale: "58%" } } } as never,
    { id: "", kind: "organization", label: "n", value: "无比例公司", source: "rb", confidence: 0.8, status: "", evidence: {} } as never,
  ]);
  expect(rows[0].name).toBe("平安银行股份有限公司 (58%)");
  expect(rows[1].name).toBe("无比例公司");
});
```

**验证：**
```bash
just test-fe -- ScopeReviewTable 2>&1 | tail -15
```
预期：新增用例通过。

**提交：** `feat(scope-review): candidatesToUnitRows maps DB candidates to review rows`

---

## Task 2 — parseReviewContext 容错 + 解析 organization_id

**文件：** `frontend/components/AIChatPanel/AskHumanInline.tsx`

**步骤：**

1. 替换 `parseReviewContext`，让它返回一个判别结果：要么「按 org_id 拉 DB」，要么「直接用数组」，要么「按文本逐行」：
```ts
type ReviewSource =
  | { kind: "org"; organizationId: string }
  | { kind: "rows"; rows: unknown }
  | { kind: "bulk"; text: string };

/** Decide where the review table's initial rows come from. Priority:
 * 1) context carries an organization_id → fetch candidates from DB (robust;
 *    the agent only had to copy a small id, not a big array);
 * 2) context is (or stringifies to) an array / {items|candidates|units:[]} → use it;
 * 3) otherwise treat context as bulk text (one entry per line). */
export function parseReviewContext(context: string): ReviewSource {
  const raw = context.trim();
  if (!raw) return { kind: "rows", rows: [] };
  let v: unknown = raw;
  try {
    v = JSON.parse(raw);
    if (typeof v === "string") v = JSON.parse(v); // tolerate double-encoded
  } catch {
    return { kind: "bulk", text: raw };
  }
  const obj = v as Record<string, unknown> | null;
  const orgId = obj && typeof obj === "object" && !Array.isArray(obj)
    ? (obj.organization_id ?? obj.organizationId)
    : undefined;
  if (typeof orgId === "string" && orgId.trim()) {
    return { kind: "org", organizationId: orgId.trim() };
  }
  if (Array.isArray(v)) return { kind: "rows", rows: v };
  if (obj && typeof obj === "object") {
    const arr = obj.items ?? obj.candidates ?? obj.units ?? obj.organizations;
    if (Array.isArray(arr)) return { kind: "rows", rows: arr };
  }
  return { kind: "bulk", text: raw };
}
```

2. 在 `AskHumanInline` 组件里，为 review 表算初始行（org → 异步拉 DB；其余同步）：
```tsx
import { useEffect, useState } from "react";
import { listOrganizationCandidates } from "@/lib/api/organizations"; // 确认导出名
import { candidatesToUnitRows } from "./ScopeReviewTable";
// ...
const reviewSource = isReviewTable ? parseReviewContext(request.context) : { kind: "rows", rows: [] };
const [dbRows, setDbRows] = useState<ScopeReviewRow[] | null>(null);
useEffect(() => {
  if (reviewSource.kind !== "org") return;
  let alive = true;
  listOrganizationCandidates(reviewSource.organizationId)
    .then((c) => { if (alive) setDbRows(candidatesToUnitRows(c.organizations)); })
    .catch(() => { if (alive) setDbRows([]); });
  return () => { alive = false; };
}, [reviewSource.kind, reviewSource.kind === "org" ? reviewSource.organizationId : ""]);
```
   渲染 `ScopeReviewTable` 时：
```tsx
<ScopeReviewTable
  kind={request.inputType as "scope_review" | "unit_review"}
  initial={
    reviewSource.kind === "org"
      ? (dbRows ?? [])
      : reviewSource.kind === "rows"
        ? reviewSource.rows
        : parseBulkRows(request.inputType as ScopeReviewKind, reviewSource.text)
  }
  onConfirm={(rows) => onSubmit(JSON.stringify(rows))}
  onSkip={onSkip}
/>
```
   （`parseBulkRows` 从 ScopeReviewTable 导出，已存在。org 模式 dbRows=null 时先给 [] 即空行，拉到后由 ScopeReviewTable 的 initial 重渲染——确认 ScopeReviewTable 用 `initial` 做 `useState` 初值；若是，用 `key={dbRows?length}` 强制重挂载或把 initial 变 controlled。见注意。）

3. 更新该文件已有的 parseReviewContext 单测（若有）为新返回结构。

**验证：**
```bash
just test-fe -- AskHumanInline 2>&1 | tail -15
just check-fe 2>&1 | tail -15
```
预期：用例通过；biome+tsc 干净。

**提交：** `feat(ask-human): unit_review sources candidates from DB by org_id; tolerant context parse`

---

## Task 3 — ScopeReviewTable initial 受控（拉到 DB 行后重渲染）

**文件：** `frontend/components/AIChatPanel/ScopeReviewTable.tsx`

**说明：** ScopeReviewTable 目前用 `initial` 做 `useState` 初值，异步 dbRows 到达不会刷新。最小改动：给 `ScopeReviewTable` 在 `AskHumanInline` 里加 `key`，dbRows 变化时重挂载。

**步骤：** 在 Task 2 的 `<ScopeReviewTable .../>` 上加：
```tsx
key={reviewSource.kind === "org" ? `org-${dbRows ? dbRows.length : "loading"}` : "ctx"}
```
（dbRows 从 null→数组时 key 变 → 组件重挂载 → 用新 initial。简单可靠，无需把 ScopeReviewTable 改成受控。）

**验证：**
```bash
just test-fe -- ScopeReviewTable AskHumanInline 2>&1 | tail -15
```
预期：通过。

**提交：** `fix(scope-review): remount review table when DB candidates arrive`

---

## Task 4 — ask_human schema 文案 + methodology

**文件：** `backend/crates/golish-agent-kit/src/tool_definitions/definitions.rs`、`resources/harness/stages/scoping.methodology.md`

**步骤：**

1. definitions.rs：把 `context` 参数描述改清楚：
```rust
                "context": {
                    "type": "string",
                    "description": "Free-form context for most input types. For 'unit_review' pass a small JSON object {\"organization_id\":\"<uuid>\"} — the review table loads that org's discovered candidates from the DB itself (do NOT hand-copy the candidate array). A JSON array of {\"name\":...} items is also accepted as a fallback. For 'scope_review' pass the target items as a JSON array."
                }
```

2. scoping.methodology.md step 3：把 context 从「候选 JSON 数组」改成「org_id 对象」：
```markdown
3. **Show the discovered subsidiaries and let the user PICK — never auto-add.**
   Call `ask_human(input_type="unit_review", context="{\"organization_id\":\"<root org id>\"}")`.
   The review table loads that org's discovered candidates from the DB and shows
   them (with ownership) for the user to confirm/edit — you do NOT need to copy the
   candidate list yourself. (No subsidiaries in scope? skip this.)
```

**验证：**
```bash
cd backend && cargo build -p golish-agent-kit 2>&1 | tail -5
rg -n "organization_id" resources/harness/stages/scoping.methodology.md | head
```
预期：build 0；methodology 含 organization_id 行。

**提交：** `docs(harness)+feat(ask-human): unit_review carries org_id so the table self-loads candidates`

---

## Task 5 — 全量验证

```bash
just check-fe 2>&1 | tail -15           # biome + tsc
just test-fe 2>&1 | tail -20            # vitest 全绿
cd backend && cargo build -p golish-agent-kit 2>&1 | tail -5
```
端到端（需重启 `just dev` 让 .md + Rust 生效；前端热更）：「搞一下平安」→ 问阈值 → discover → **unit_review 表自动列出子公司候选（带投资比）给你勾**（不再空/占位符），不管 LLM 在 context 里塞了什么。

---

## 自检
1. **根因**：表格空 = LLM 没把数组塞进 context。本计划让表格**从 DB 取数**（agent 只传 org_id）→ 不再依赖 LLM 搬数组。✅ 容错解析兜底其它格式。✅
2. **占位符扫描**：无 TODO；每步带代码。✅（两处「确认导出名」是让执行者核对 `OrganizationCandidate` / `listOrganizationCandidates` 的实际导出，已给出备选路径，非占位。）
3. **类型一致**：`parseReviewContext` 返回 `ReviewSource` 判别联合，Task2 消费一致；`candidatesToUnitRows(OrganizationCandidate[])→ScopeReviewRow[]` Task1 定义 Task2 调用一致。✅

## 注意
- discover 必须 `create_candidates=true`（run_phase 已设）才会把候选落库供 `organization_candidates_list` 取——已满足。
- org 模式异步拉数时表格先空、拉到后经 `key` 重挂载填充；可加一行 loading 文案（可选）。
- 仍保留「context 是数组/文本」的兜底，向后兼容 scope_review 与旧调用。

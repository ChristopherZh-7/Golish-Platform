# P0-4 前端调用层回归 api 层（裸 invoke 收口）实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 `.cursor/skills/executing-plans/` 逐任务实现此计划；每个任务单独 commit。
> Relates to: `docs/design/2026-05-29-architecture-optimization.md` §4.2 / §5 P0-4；AGENTS.md 不变量 I5、规范 §2.3（前端禁裸 `invoke()`）、`docs/development.md#adding-a-new-tauri-command-ipc`。

**目标：** 前端组件层不得直接调用 Tauri `invoke`，所有 IPC 必须走 `frontend/lib/api/<domain>.ts` 的类型化包装；并把错放在 `lib/api/targets.ts` 的 pipeline 写操作归位到 `lib/api/pipeline.ts`，最后用 biome + barrel 收口防止回潮。

**架构：** 前端有一条既定的 IPC 分层——组件 → `@/lib/api/<domain>`（域包装）→ `client.invoke`（`frontend/lib/api/client.ts:55`，统一 traceId + ApiError）→ Tauri。本计划不新增机制，只把**绕过这条链路**的调用点搬回域包装层；其中大多数域包装**已经存在**（尤其 `vulnIntelApi`），组件只是没用。

**技术栈：** TypeScript 6、React 19、`@/lib/api` barrel、biome 2.3（`noRestrictedImports`）、Vitest、`just`、`rg`、git。

---

## 范围（重要）

本计划**只覆盖** P0-4 的「调用层回归」一半：消除组件层裸 `invoke`、归位 pipeline 写操作、加 lint 收口。

设计文档 §5 P0-4 还提到「为 `FindingsPanel`/`PipelinePanel`/`ProjectOverview` 补 error 态」，那是**三态 UI**问题（§4.1 / I1 错误码契约），与调用层是独立子系统，**不在本计划内**，按 writing-plans「范围检查」拆为独立计划另行推进（建议 `docs/superpowers/plans/2026-05-29-p0-panel-error-states.md`）。本计划仅在不改变行为的前提下原样保留各调用点现有的 `.catch()` 处理。

---

## 现状（事实，带证据）

### A. 组件层裸 `invoke` 调用清单（`rg -n 'invoke[<(]' frontend/components`，已剔除 `*.test.*`）

| # | 文件:行 | 当前调用 | 已有域包装？ | 目标包装 |
|---|---|---|---|---|
| 1 | `components/PipelinePanel/PipelinePanel.tsx:108` | `invoke<Pipeline[]>("pipeline_list", { projectPath })` | 有 | `pipeline.listPipelines(projectPath)`（`lib/api/pipeline.ts:11`） |
| 2 | `components/VulnIntelPanel/PocTab.tsx:73` | `invoke<GithubPocResult[]>("intel_search_github_poc", { cveId })` | 有 | `vulnIntelApi.searchGithubPoc(cveId)`（vuln-intel.ts:86） |
| 3 | `components/VulnIntelPanel/PocTab.tsx:87` | `invoke<NucleiTemplateResult[]>("intel_search_nuclei_templates", { cveId })` | 有 | `vulnIntelApi.searchNucleiTemplates(cveId)`（vuln-intel.ts:88） |
| 4 | `components/VulnIntelPanel/PocTab.tsx:104` | `invoke<PocTemplate>("vuln_link_add_poc_full", {...})` | **无** | 新增 `vulnIntelApi.addPocFull(...)` |
| 5 | `components/VulnIntelPanel/PocTab.tsx:374` | `invoke<PocTemplate>("vuln_link_add_poc", {...})` | 有 | `vulnIntelApi.addPocFromSource(...)`（vuln-intel.ts:90） |
| 6 | `components/VulnIntelPanel/PocLibraryView.tsx:99` | `invoke<NucleiDiscoverResult>("intel_discover_all_nuclei")` | 有 | `vulnIntelApi.discoverAllNuclei()`（vuln-intel.ts:114） |
| 7 | `components/VulnIntelPanel/PocLibraryView.tsx:105` | `invoke<Record<string, DbVulnLinkFull>>("vuln_link_get_all")` | 有 | `vulnIntelApi.getAllLinks()`（vuln-intel.ts:74） |
| 8 | `components/VulnIntelPanel/VulnDetailView.tsx:49` | `invoke<...>("kb_research_load", { cveId })` | 有 | `vulnIntelApi.researchLoad(cveId)`（vuln-intel.ts:117） |
| 9 | `components/VulnIntelPanel/VulnDetailView.tsx:61` | `invoke<DbVulnLinkFull>("vuln_link_get", { cveId })` | 有 | `vulnIntelApi.getLink(cveId)`（vuln-intel.ts:75） |
| 10 | `components/VulnIntelPanel/useWikiTab.ts:162` | `invoke<WikiPageInfo[]>("wiki_pages_for_paths", { paths })` | 有 | `vulnIntelApi.wikiPagesForPaths(paths)`（vuln-intel.ts:128） |
| 11 | `components/VulnIntelPanel/useWikiTab.ts:168` | `invoke<WikiPageInfo[]>("wiki_suggest_for_cve", { cveId, limit: 8 })` | 有 | `vulnIntelApi.wikiSuggestForCve(cveId, 8)`（vuln-intel.ts:129） |
| 12 | `components/VulnIntelPanel/useWikiTab.ts:178` | `invoke<WikiBacklinkInfo[]>("wiki_backlinks", { path })` | 有 | `vulnIntelApi.wikiBacklinks(path)`（vuln-intel.ts:131） |
| 13 | `components/VulnIntelPanel/useWikiTab.ts:301` | `invoke<...>("wiki_search_db", { query, limit: 20 })` | **无** | 新增 `vulnIntelApi.wikiSearchDb(query, 20)` |

> 结论：13 处里 **11 处的域包装已存在**，组件只是绕过；仅 2 处（#4、#13）需要在 `vuln-intel.ts` 新增包装。`PipelinePanel`、`PocTab`、`useWikiTab` 均通过 `import { invoke } from "@/lib/api"`（barrel 在 `lib/api/index.ts:14` re-export 了 `invoke`）拿到裸 `invoke`。

### B. pipeline 写操作错放在 `targets.ts`

`lib/api/pipeline.ts:1-6` 头注释自述：写操作「currently live in `lib/api/targets.ts` for historical reasons — consolidate in a follow-up PR」。证据：

- `lib/api/targets.ts:59-65` `executePipeline`（`pipeline_execute`）
- `lib/api/targets.ts:67-69` `cancelPipeline`（`pipeline_cancel`）
- `lib/api/targets.ts:71-73` `deletePipeline`（`pipeline_delete`）

调用方（`rg -n 'executePipeline|cancelPipeline|deletePipeline' frontend`）：

- `components/TargetPanel/hooks/usePipelineForm.ts:151` → `targets.executePipeline(...)`
- `components/TargetPanel/hooks/usePipelineForm.ts:165` → `targets.cancelPipeline()`
- `components/PipelinePanel/PipelinePanel.tsx:174` → `targets.deletePipeline(id, ...)`

### C. biome 现状与缺口

`biome.json:52-60` 已对组件层禁了两条 import：

- `@tauri-apps/api/core`（level error）
- `@/lib/api/client`（level error）

`biome.json:74-88` override 对 `frontend/lib/**` 与测试文件关闭该规则。

**缺口**：barrel `@/lib/api` re-export 了 `invoke`（`lib/api/index.ts:14`），而 `@/lib/api` **不在** `noRestrictedImports.paths` 里，所以组件 `import { invoke } from "@/lib/api"` 不被拦截——这正是 #1~#13 的入口。生产组件目前**没有**任何 `@tauri-apps/api/core` 的 `invoke` 直接 import（仅 `client.ts:1` 的 `tauriInvoke` 与 3 个 `*.test.*`），所以收口只需堵 barrel 这一个洞。

### D. 已具备的共享件

- `lib/api/client.ts:55` `invoke<T>()`：唯一允许调 Tauri 的封装（traceId + ApiError）。
- `lib/api/vuln-intel.ts:54-142` `vulnIntelApi`：vuln-intel 域包装集合（已含 #2/#3/#5/#6/#7/#8/#9/#10/#11/#12 的方法）。

---

## 文件结构（创建 / 修改 + 职责）

| 文件 | 动作 | 职责 |
|---|---|---|
| `frontend/lib/api/pipeline.ts` | 修改 | 接收 `executePipeline`/`cancelPipeline`/`deletePipeline` 三个写操作包装 |
| `frontend/lib/api/targets.ts` | 修改 | 删除上述三个 pipeline 写操作（targets 域不再越界持有 pipeline IPC） |
| `frontend/components/TargetPanel/hooks/usePipelineForm.ts` | 修改 | 调用方从 `targets.*` 切到 `pipeline.*` |
| `frontend/components/PipelinePanel/PipelinePanel.tsx` | 修改 | `:108` 改用 `listPipelines`；`:174` 改用 `deletePipeline`；移除 `invoke` import |
| `frontend/lib/api/vuln-intel.ts` | 修改 | 新增 `addPocFull` 与 `wikiSearchDb` 两个缺失包装 |
| `frontend/components/VulnIntelPanel/PocTab.tsx` | 修改 | 4 处裸 invoke → `vulnIntelApi.*`，移除 `invoke` import |
| `frontend/components/VulnIntelPanel/PocLibraryView.tsx` | 修改 | 2 处裸 invoke → `vulnIntelApi.*`，移除 `invoke` import |
| `frontend/components/VulnIntelPanel/VulnDetailView.tsx` | 修改 | 2 处裸 invoke → `vulnIntelApi.*`，移除 `invoke` import |
| `frontend/components/VulnIntelPanel/useWikiTab.ts` | 修改 | 4 处裸 invoke → `vulnIntelApi.*`，移除 `invoke` import |
| `frontend/lib/api/index.ts` | 修改 | barrel 不再 re-export `invoke`（收口） |
| `biome.json` | 修改 | `noRestrictedImports` 增 `@/lib/api` 的 `invoke` 限制（双保险） |

> **DRY / YAGNI**：优先复用 `vulnIntelApi` 既有方法，只在确实缺失时（#4/#13）加包装；不顺手重构无关的 vuln-intel 类型或组件逻辑（AGENTS.md §3「不引入 scope 外改动」）。

---

## 任务分解（小步骤）

### 任务 1：pipeline 写操作从 `targets.ts` 归位 `pipeline.ts`

- **文件：** `frontend/lib/api/pipeline.ts`、`frontend/lib/api/targets.ts`
- **步骤：**
  1. 在 `frontend/lib/api/pipeline.ts` 末尾（`listPipelineTemplates` 之后）追加三个写操作，并把头注释里「写操作在 targets.ts」那段删掉：

```ts
export async function executePipeline(params: {
  pipeline: unknown;
  target: string;
  projectPath: string | null;
}): Promise<void> {
  await invoke("pipeline_execute", params);
}

export async function cancelPipeline(): Promise<void> {
  await invoke("pipeline_cancel");
}

export async function deletePipeline(id: string, projectPath: string | null): Promise<void> {
  await invoke("pipeline_delete", { id, projectPath });
}
```

  2. 把 `frontend/lib/api/pipeline.ts:1-6` 的头注释改为（去掉「写操作在 targets.ts、待收拢」一段）：

```ts
/**
 * Pipeline IPC wrappers — read-side (list/save/templates) and write-side
 * (execute/cancel/delete). All pipeline IPC lives here.
 */
```

  3. 从 `frontend/lib/api/targets.ts` 删除 `executePipeline`（59-65）、`cancelPipeline`（67-69）、`deletePipeline`（71-73）三段函数。
- **验证：**
  - `pnpm typecheck`（或 `just check-fe`）——此时 `usePipelineForm.ts` / `PipelinePanel.tsx` 仍引用 `targets.executePipeline` 等会**编译报错**，属预期，由任务 2 修复。可先只编译 `lib/api`：确认 `pipeline.ts`/`targets.ts` 自身无语法错。
- **提交：** `git commit -m "refactor(api): move pipeline write ops from targets.ts to pipeline.ts"`（与任务 2 连续提交，避免中间态滞留；若要单测绿，可与任务 2 合并为一个 commit）。

### 任务 2：更新 pipeline 写操作的 3 个调用方

- **文件：** `frontend/components/TargetPanel/hooks/usePipelineForm.ts`、`frontend/components/PipelinePanel/PipelinePanel.tsx`
- **步骤：**
  1. `usePipelineForm.ts:2` 已是 `import { pipeline, targets } from "@/lib/api";`。把 `:151` 的 `targets.executePipeline({...})` 改为 `pipeline.executePipeline({...})`，`:165` 的 `targets.cancelPipeline()` 改为 `pipeline.cancelPipeline()`。
  2. 若改完后 `targets` 在 `usePipelineForm.ts` 内不再被使用，则从 `:2` import 删除 `targets`（biome `noUnusedImports` 会 warn）；先 `rg -n '\btargets\.' frontend/components/TargetPanel/hooks/usePipelineForm.ts` 确认。
  3. `PipelinePanel.tsx:19` 现为 `import { listPipelineTemplates, savePipeline, savePipelineTemplate } from "@/lib/api/pipeline";`。改为：

```ts
import {
  deletePipeline,
  listPipelines,
  listPipelineTemplates,
  savePipeline,
  savePipelineTemplate,
} from "@/lib/api/pipeline";
```

  （`listPipelines` 供任务 3 使用，一并加入。）
  4. `PipelinePanel.tsx:174` 把 `await targets.deletePipeline(id, getProjectPath());` 改为 `await deletePipeline(id, getProjectPath());`。
- **验证：** `pnpm typecheck`；`rg -n 'targets\.(execute|cancel|delete)Pipeline' frontend` 预期 **0 命中**。
- **提交：** 与任务 1 同一逻辑变更，建议合并提交：`git commit -m "refactor(api): route pipeline write ops through pipeline.ts"`。

### 任务 3：`PipelinePanel.tsx:108` 裸 invoke → `listPipelines`

- **文件：** `frontend/components/PipelinePanel/PipelinePanel.tsx`
- **步骤：**
  1. `:108` 把 `invoke<Pipeline[]>("pipeline_list", { projectPath: getProjectPath() })` 改为 `listPipelines(getProjectPath())`：

```ts
    const [pl, tl, ai] = await Promise.all([
      listPipelines(getProjectPath()),
      scanTools(),
      listAiTools().catch(() => [] as AiToolMeta[]),
    ]);
```

  > 注意类型：`listPipelines` 返回 `PipelineSummary[]`（`lib/api/pipeline.ts:11`），而原 `invoke` 标注为 `Pipeline[]`。后端命令 `pipeline_list` 同一个，差异仅是前端类型标注。沿用包装的 `PipelineSummary[]`；若 `setPipelines` 的 state 类型当前是 `Pipeline[]`，将其 state 类型同步为 `PipelineSummary[]`（与 `usePipelineForm.ts:17` 一致），并据 typecheck 报错点收敛 `active`/列表渲染处的字段引用。
  2. `:18` `import { invoke, targets } from "@/lib/api";`：移除 `invoke`。`targets` 在任务 2 后若已无用（`deletePipeline` 已切走），整行删除；否则保留 `targets`。先 `rg -n '\b(invoke|targets)\b' frontend/components/PipelinePanel/PipelinePanel.tsx` 核对。
- **验证：** `pnpm typecheck`；`rg -n 'invoke[<(]' frontend/components/PipelinePanel` 预期 0 命中。
- **提交：** `git commit -m "refactor(pipeline-panel): use listPipelines wrapper instead of raw invoke"`。

### 任务 4：`vuln-intel.ts` 新增 2 个缺失包装

- **文件：** `frontend/lib/api/vuln-intel.ts`
- **步骤：**
  1. 在 `vulnIntelApi` 的 PoC search 区（`discoverAllNuclei` 之后，约 `:114`）新增 `addPocFull`（镜像 `addPocFromSource`，命令名换成 `vuln_link_add_poc_full`）：

```ts
  // Import a full PoC (e.g. a Nuclei template) as-is.
  addPocFull: (params: {
    cveId: string;
    name: string;
    pocType: string;
    language: string;
    content: string;
    source: string;
    sourceUrl: string;
    severity: string;
    description: string;
    tags: string[];
  }) => invoke<PocTemplate>("vuln_link_add_poc_full", params),
```

  2. 在 Wiki 区（`wikiBacklinks` 之后，约 `:131`）新增 `wikiSearchDb`：

```ts
  wikiSearchDb: (query: string, limit: number) =>
    invoke<
      Array<{
        path: string;
        title: string;
        category: string;
        tags: string[];
        status: string | null;
      }>
    >("wiki_search_db", { query, limit }),
```

- **验证：** `pnpm typecheck`（仅新增导出，应直接绿）。
- **提交：** `git commit -m "feat(api): add vulnIntelApi.addPocFull and wikiSearchDb wrappers"`。

### 任务 5：迁移 `VulnIntelPanel/PocTab.tsx`（4 处）

- **文件：** `frontend/components/VulnIntelPanel/PocTab.tsx`
- **步骤：**
  1. `:73` `invoke<GithubPocResult[]>("intel_search_github_poc", { cveId })` → `vulnIntelApi.searchGithubPoc(cveId)`。
  2. `:87` `invoke<NucleiTemplateResult[]>("intel_search_nuclei_templates", { cveId })` → `vulnIntelApi.searchNucleiTemplates(cveId)`。
  3. `:104` 改用任务 4 新增的 `addPocFull`：

```ts
      const dbPoc = await vulnIntelApi.addPocFull({
        cveId,
        name: `[Nuclei] ${template.name}`,
        pocType: "nuclei",
        language: "yaml",
        content: template.content,
        source: "nuclei_template",
        sourceUrl: template.html_url,
        severity: template.severity ?? "unknown",
        description: "",
        tags: [],
      });
```

  4. `:374` `invoke<PocTemplate>("vuln_link_add_poc", {...})` → `vulnIntelApi.addPocFromSource(cveId, name, type, language, content, source, sourceUrl, severity, description, tags)`（按 `vuln-intel.ts:90-113` 的位置参数顺序传入该调用点现有的同名变量）。
  5. `:18` `import { invoke } from "@/lib/api";`：删除该行（`vulnIntelApi` 已在 `:19` 导入）。
- **验证：** `pnpm typecheck`；`rg -n 'invoke[<(]' frontend/components/VulnIntelPanel/PocTab.tsx` 预期 0 命中。
- **提交：** `git commit -m "refactor(vuln-intel): PocTab uses vulnIntelApi wrappers"`。

### 任务 6：迁移 `VulnIntelPanel/PocLibraryView.tsx`（2 处）

- **文件：** `frontend/components/VulnIntelPanel/PocLibraryView.tsx`
- **步骤：**
  1. `:99` `invoke<NucleiDiscoverResult>("intel_discover_all_nuclei")` → `vulnIntelApi.discoverAllNuclei()`。
  2. `:105` `invoke<Record<string, DbVulnLinkFull>>("vuln_link_get_all")` → `vulnIntelApi.getAllLinks()`。
  3. 移除 `invoke` 的 import（确认 `vulnIntelApi` 已导入；若没有则加 `import { vulnIntelApi } from "@/lib/api/vuln-intel";`）。
- **验证：** `pnpm typecheck`；`rg -n 'invoke[<(]' frontend/components/VulnIntelPanel/PocLibraryView.tsx` 预期 0 命中。
- **提交：** `git commit -m "refactor(vuln-intel): PocLibraryView uses vulnIntelApi wrappers"`。

### 任务 7：迁移 `VulnIntelPanel/VulnDetailView.tsx`（2 处）

- **文件：** `frontend/components/VulnIntelPanel/VulnDetailView.tsx`
- **步骤：**
  1. `:49` `invoke<...>("kb_research_load", { cveId: entry.cve_id })` → `vulnIntelApi.researchLoad(entry.cve_id)`。
  2. `:61` `invoke<DbVulnLinkFull>("vuln_link_get", { cveId: entry.cve_id })` → `vulnIntelApi.getLink(entry.cve_id)`。
  3. 移除 `invoke` 的 import（确认 `vulnIntelApi` 已导入；否则补 `import { vulnIntelApi } from "@/lib/api/vuln-intel";`）。
- **验证：** `pnpm typecheck`；`rg -n 'invoke[<(]' frontend/components/VulnIntelPanel/VulnDetailView.tsx` 预期 0 命中。
- **提交：** `git commit -m "refactor(vuln-intel): VulnDetailView uses vulnIntelApi wrappers"`。

### 任务 8：迁移 `VulnIntelPanel/useWikiTab.ts`（4 处）

- **文件：** `frontend/components/VulnIntelPanel/useWikiTab.ts`
- **步骤：**
  1. `:162` `invoke<WikiPageInfo[]>("wiki_pages_for_paths", { paths: link.wikiPaths })` → `vulnIntelApi.wikiPagesForPaths(link.wikiPaths)`。
  2. `:168` `invoke<WikiPageInfo[]>("wiki_suggest_for_cve", { cveId, limit: 8 })` → `vulnIntelApi.wikiSuggestForCve(cveId, 8)`。
  3. `:178` `invoke<WikiBacklinkInfo[]>("wiki_backlinks", { path: selectedPath })` → `vulnIntelApi.wikiBacklinks(selectedPath)`。
  4. `:301` 改用任务 4 新增的 `wikiSearchDb`：

```ts
      const results = await vulnIntelApi.wikiSearchDb(query.trim(), 20);
```

  5. 移除 `invoke` 的 import（`vulnIntelApi` 已用于 `:223` 等处，无需新增）。
- **验证：** `pnpm typecheck`；`rg -n 'invoke[<(]' frontend/components/VulnIntelPanel/useWikiTab.ts` 预期 0 命中。
- **提交：** `git commit -m "refactor(vuln-intel): useWikiTab uses vulnIntelApi wrappers"`。

### 任务 9：收口——barrel 停止 re-export `invoke` + biome 兜底

> 必须是**最后一步**：前置任务把全部组件调用点迁完后才能堵洞，否则编译会断在未迁移的调用点。

- **文件：** `frontend/lib/api/index.ts`、`biome.json`
- **步骤：**
  1. `frontend/lib/api/index.ts:14` 现为：

```ts
export { ApiError, getInflightCommands, invoke } from "./client";
```

  改为（移除 `invoke`，组件不再能从 barrel 取裸 invoke；`lib/api/*` 内部一律 `import { invoke } from "./client"`，不受影响）：

```ts
export { ApiError, getInflightCommands } from "./client";
```

  2. 在 `biome.json` 的 `linter.rules.style.noRestrictedImports.options.paths`（`:55-58`）增加一条（双保险，防止有人重新从 barrel 导出 invoke）：

```json
"@tauri-apps/api/core": "Direct invoke() from Tauri is forbidden outside frontend/lib/api/. Use api.<domain>.<verb> from @/lib/api or a typed wrapper from @/lib/api/<domain>. See docs/development.md#adding-a-new-tauri-command-ipc.",
"@/lib/api/client": "Direct facade-client invoke() from outside frontend/lib/api/ is discouraged. Use api.<domain>.<verb> from @/lib/api or a typed wrapper.",
"@/lib/api": "Importing `invoke` from the api barrel is forbidden in components. Use api.<domain>.<verb> or a typed wrapper from @/lib/api/<domain>."
```

  > 说明：biome 的 `noRestrictedImports` 现版本对某路径只能给单条消息（整路径级）。直接禁整个 `@/lib/api` 会误伤合法的域 namespace import。**优先以步骤 1（barrel 不再导出 invoke）作为硬约束**（tsc 即报错）；biome 这条仅在未来需要更强约束时启用。若启用导致大量误报，则回退本步骤 2，仅保留步骤 1。
- **验证：**
  - `rg -n 'invoke[<(]' frontend/components` 预期 **0 命中**（最终目标）。
  - `rg -n 'from "@/lib/api"' frontend/components | rg '\binvoke\b'` 预期 0 命中。
  - `just check-fe`（biome + typecheck）通过。
- **提交：** `git commit -m "chore(api): stop re-exporting invoke from api barrel to enforce wrapper layer"`。

---

## 验证（统一）

| 命令 | 用途 | 预期 |
|---|---|---|
| `just check-fe`（= biome + `tsc --noEmit`） | 静态门禁 | 全绿，无类型/lint 错误 |
| `just test-fe`（Vitest） | 前端单测 | 全绿（本计划不改业务逻辑，行为不变） |
| `rg -n 'invoke[<(]' frontend/components` | 组件层裸 invoke 兜底 | **0 命中** |
| `rg -n 'targets\.(execute\|cancel\|delete)Pipeline' frontend` | pipeline 写操作归位校验 | 0 命中 |
| `git grep -n 'invoke' frontend/lib/api/index.ts` | barrel 不再导出 invoke | 仅注释/无 export |

> 完成定义（对齐 AGENTS.md §3）：上述命令**实际跑过**、输出抄进 `agent-progress.md` 的「已记录证据」，并把 `feature_list.json` 对应条目 `verification` 逐条核对、填 `evidence`。没有新鲜证据不许标 `passing`。

---

## 风险与回滚

| 风险 | 说明 | 缓解 |
|---|---|---|
| 类型标注差异 | `pipeline_list` 包装返回 `PipelineSummary[]`，原组件标 `Pipeline[]`（任务 3） | 以包装类型为准，按 typecheck 报错点收敛字段引用；二者来自同一后端命令，运行时一致 |
| 位置参数传错 | `addPocFromSource` 是 10 个位置参数（任务 5.4） | 严格按 `vuln-intel.ts:90-113` 顺序；改完 typecheck 校验 |
| 收口顺序 | 任务 9 早于迁移完成会断编译 | 强制最后执行；前 8 任务跑完 `rg` 确认 0 残留再做 |
| `targets` 残留未用 import | 删 pipeline 写操作后 `targets` 可能变未用 | `rg '\btargets\.'` 核对后再删 import，biome `noUnusedImports` 兜底 |
| scope 蔓延 | 迁移时顺手改 vuln-intel 业务逻辑 | 严守 AGENTS.md §3，仅换调用方式、不动逻辑与 `.catch` |

**统一回滚原则**：每个组件/文件一个 commit，纯调用层替换，任一可独立 `git revert`；barrel 收口（任务 9）单独 commit，回退即恢复 `invoke` 导出。

---

## 自检

**1. 规格覆盖度**（对照 agent-1 任务 §分步 ①-④ 与设计 §5 P0-4）：

- ① 扫描全前端裸 invoke 清单 → 「现状 §A」13 行表 + 「§B」写操作 + 「§C」biome 缺口 ✅
- ② 补齐/归位 `lib/api/pipeline.ts` 等封装 → 任务 1（归位）+ 任务 4（补 2 个 vuln-intel 包装）✅
- ③ 组件改调 api 层 → 任务 2/3（pipeline）+ 任务 5/6/7/8（vuln-intel）✅
- ④（可选）加 lint 规则禁组件直接 invoke → 任务 9（barrel 硬约束 + biome 兜底）✅
- 影响面（`pipeline.ts`/`targets.ts`/`PipelinePanel` 等调用方）→ 「文件结构」表全覆盖 ✅
- 验证（`just check-fe`/`test-fe` + grep 无裸 invoke）→ 「验证」表 ✅
- 回滚、风险 → 已列 ✅
- 非目标：error 态补齐已显式排除（见「范围」），归 sibling 计划。

**2. 占位符扫描**：无「TODO/待定/类似任务 N」；#4、#13 的新包装与所有迁移点均给出实际代码或精确的「文件:行 + 命令名 + 目标方法」。`addPocFromSource`（任务 5.4）以「按既有 10 参顺序传现有变量」描述而非贴整段——因其变量名属调用点上下文，执行者据 typecheck 即可对齐。

**3. 类型一致性**：新增 `addPocFull` 入参对象与 `vuln_link_add_poc_full` 现调用点（`PocTab.tsx:104-115`）字段逐一对应；`wikiSearchDb` 返回类型复制自 `useWikiTab.ts:301-303` 现有内联标注；`listPipelines`/`deletePipeline`/`executePipeline`/`cancelPipeline` 签名与 `targets.ts` 原定义逐字一致，仅迁移位置。

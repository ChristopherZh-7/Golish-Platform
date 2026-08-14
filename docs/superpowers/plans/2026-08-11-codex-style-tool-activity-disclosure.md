# Codex 式工具活动与命令披露实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 把 Company Controller selected-Agent transcript 的 raw JSON 工具卡改成 Codex 式活动摘要，并允许逐层展开到实际命令、输出和 Raw Tool Data。
**架构：** 新建纯函数 presentation adapter 统一解析 `SubAgentToolCall`；`StageTeamWorkspaceView` 只负责 transcript 分组和 disclosure UI。命令只消费后端实际字段，未知/非命令工具稳定退回 raw data，不推导 Gate、coverage 或 evidence truth。
**技术栈：** React 19、TypeScript 6、Vitest、Testing Library、Tailwind 4、现有 `SubAgentToolCall` store 类型。

## 文件结构

- 新建 `frontend/components/Engagement/toolActivityPresentation.ts`：object/JSON-string 归一化、动作/runner/subject、真实 command/output/job/hint 提取、活动组标题。
- 新建 `frontend/components/Engagement/toolActivityPresentation.test.ts`：presentation adapter 的纯单测。
- 新建 `frontend/components/Engagement/ToolActivityDisclosure.tsx`：分组、工具、terminal 与 Raw Data 三层 disclosure 组件。
- 修改 `frontend/components/Engagement/StageTeamWorkspaceView.tsx`：连续 generic tools 分组、两层 disclosure、terminal/raw 渲染。
- 修改 `frontend/components/Engagement/StageTeamWorkspaceView.test.tsx`：完整交互与状态回归。
- 修改 `docs/modules/frontend/components.md` 与 `docs/modules/INDEX.md`：记录新的单一展示合同。
- 修改 `feature_list.json` 与 `agent-progress.md`：active feature、验证证据和交接状态。

## 任务 1：为 presentation adapter 写 RED 测试

**文件：**
- 创建：`frontend/components/Engagement/toolActivityPresentation.test.ts`

**步骤 1：** 写 object 与 JSON-string 两种 result 的测试，精确要求读取真实 command 和 partial output：

```ts
expect(
  presentToolActivity({
    id: "ports",
    name: "eas_discover_ports",
    args: { targets: ["192.0.2.10"], scan_profile: "standard" },
    status: "backgrounded",
    result: {
      command: "naabu -list /tmp/input -top-ports 1000",
      partial_stdout: "192.0.2.10:443\n",
      job_id: "job_ports",
      hint: "Managed process is still running",
    },
    startedAt: "2026-08-11T00:00:00Z",
  }).command
).toBe("naabu -list /tmp/input -top-ports 1000");
```

**步骤 2：** 写非命令工具不伪造 command、streamingOutput 优先于 result stdout、动作组标题按首次出现去重的测试。

**步骤 3：** 运行并确认 RED：

```bash
pnpm exec vitest run frontend/components/Engagement/toolActivityPresentation.test.ts
```

预期：模块不存在或导出不存在，命令退出非零。

**提交：** 共享工作树已有未提交状态文件；本任务不自动 commit，只记录 RED 命令和输出。

## 任务 2：实现最小 presentation adapter

**文件：**
- 创建：`frontend/components/Engagement/toolActivityPresentation.ts`

**步骤 1：** 定义稳定 view model 与安全 object 归一化：

```ts
export interface ToolActivityPresentation {
  action: string;
  completedAction: string;
  runner: string | null;
  subject: string | null;
  command: string | null;
  commandProvenance: "executed" | "requested" | null;
  stdout: string | null;
  stderr: string | null;
  hint: string | null;
  jobId: string | null;
}

function normalizedRecord(value: unknown): Record<string, unknown> | null {
  if (typeof value === "object" && value !== null && !Array.isArray(value)) return value as Record<string, unknown>;
  if (typeof value !== "string") return null;
  try {
    const parsed: unknown = JSON.parse(value);
    return typeof parsed === "object" && parsed !== null && !Array.isArray(parsed)
      ? (parsed as Record<string, unknown>)
      : null;
  } catch {
    return null;
  }
}
```

**步骤 2：** `presentToolActivity(tool)` 对 `eas_discover_ports` 返回“扫描端口/Naabu”，通用 fallback 复用 `getToolActionLabel` 与 `getToolPrimaryArg`。只读取 exact command/output/job/hint 字段；result command 标为 `executed`，仅 raw shell args command 可标为 `requested`，EAS/pentest wrapper 不做命令重建。

**步骤 3：** `summarizeToolActivities(presentations)` 按顺序去重 completed/current action，最多两个动作，更多项追加计数。

**步骤 4：** 运行 GREEN：

```bash
pnpm exec vitest run frontend/components/Engagement/toolActivityPresentation.test.ts
```

预期：全部测试通过，exit 0。

**提交：** 不自动 commit；只在用户明确要求提交时 stage 本任务文件。

## 任务 3：为嵌套 disclosure 写 RED 组件测试

**文件：**
- 修改：`frontend/components/Engagement/StageTeamWorkspaceView.test.tsx`

**步骤 1：** 构造同一 Agent 的两个连续 generic tool entries，其中 `eas_discover_ports` 为 backgrounded 且 result 含 command/job/hint，第二项为 completed。

**步骤 2：** 断言初始只显示一个活动摘要，不显示工具名、命令和 raw key；点击活动摘要后显示两个工具行但仍不显示命令；点击 Naabu 行后显示 `$ naabu ...`、job/hint 和 output；点击 Raw Tool Data 后显示 `scan_profile` 与 `completion_state`。

**步骤 3：** 断言按钮有 `aria-expanded`，backgrounded spinner 不被标成 completed；thinking/text 和 dispatch 会打断普通工具组。

**步骤 4：** 运行并确认 RED：

```bash
pnpm exec vitest run frontend/components/Engagement/StageTeamWorkspaceView.test.tsx
```

预期：找不到新活动摘要或 disclosure，命令退出非零。

**提交：** 不自动 commit；记录 RED 证据。

## 任务 4：实现活动分组与命令终端卡

**文件：**
- 创建：`frontend/components/Engagement/ToolActivityDisclosure.tsx`
- 修改：`frontend/components/Engagement/StageTeamWorkspaceView.tsx`

**步骤 1：** 在 presentation layer 把相邻 generic tool entries 收成 `{kind: "tool_activity_group", entries}`；保留 plan、dispatch、text、thinking 的既有顺序与专用 renderer。

**步骤 2：** 新增 `ToolActivityGroup`：默认 header 显示 icon、摘要、聚合状态和 disclosure；展开后逐行显示 action、runner、subject 与 status。

**步骤 3：** 新增 `ToolActivityRow`：二次 disclosure 展示真实 command、streaming/stdout/stderr、job/hint；无 command 时显示明确的“此工具没有命令行执行记录”，不伪造 shell。

**步骤 4：** 在 row 最下方添加默认折叠 `Raw Tool Data`，分别用现有 `JsonView` 显示 args/result；保留完整数据但不重复 stringify 成默认主内容。

**步骤 5：** 保持 200-entry presentation bound、Plan pinning、auto-scroll 和 Stage Agent tree 行为不变。

**步骤 6：** 运行组件 GREEN：

```bash
pnpm exec vitest run frontend/components/Engagement/StageTeamWorkspaceView.test.tsx frontend/components/Engagement/toolActivityPresentation.test.ts
```

预期：全部通过，exit 0。

**提交：** 不自动 commit；共享 dirty tree 下避免把无关后端或状态文件带入提交。

## 任务 5：定向静态验证与文档收尾

**文件：**
- 修改：`docs/modules/frontend/components.md`
- 修改：`docs/modules/INDEX.md`
- 修改：`feature_list.json`
- 修改：`agent-progress.md`

**步骤 1：** 运行 affected Biome：

```bash
pnpm exec biome check frontend/components/Engagement/toolActivityPresentation.ts frontend/components/Engagement/toolActivityPresentation.test.ts frontend/components/Engagement/ToolActivityDisclosure.tsx frontend/components/Engagement/StageTeamWorkspaceView.tsx frontend/components/Engagement/StageTeamWorkspaceView.test.tsx
```

预期：exit 0，无 warning/error。

**步骤 2：** 运行类型检查：

```bash
pnpm typecheck
```

预期：exit 0。

**步骤 3：** 运行 JSON、唯一 active feature 与 scoped diff 检查：

```bash
jq empty feature_list.json
jq -e '([.features[] | select(.status == "in_progress")] | length) <= 1' feature_list.json
git diff --check -- frontend/components/Engagement/toolActivityPresentation.ts frontend/components/Engagement/toolActivityPresentation.test.ts frontend/components/Engagement/ToolActivityDisclosure.tsx frontend/components/Engagement/StageTeamWorkspaceView.tsx frontend/components/Engagement/StageTeamWorkspaceView.test.tsx docs/design/2026-08-11-codex-style-tool-activity-disclosure.md docs/superpowers/plans/2026-08-11-codex-style-tool-activity-disclosure.md docs/modules/frontend/components.md docs/modules/INDEX.md feature_list.json agent-progress.md
```

预期：三个命令均 exit 0。

**步骤 4：** 把精确命令、退出码、测试数量、未运行的全量门禁、共享 dirty tree 风险写入 progress；只有上述 fresh evidence 全绿后才把本 feature 设为 `passing`。

**提交：** 不自动 commit；向用户报告本轮文件与共享未提交文件的边界。

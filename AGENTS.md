# AGENTS.md

> Golish 平台的 **agent 工作宪法**。任何 AI agent（Cursor Agent、Codex、Claude Code、Gemini CLI、Aider 等）在本仓库工作前必须先读这份文件，工作过程中持续遵守，工作结束前对照检查。
>
> 配套文件：`agent-progress.md`（进度日志）、`feature_list.json`（功能清单）、`init.sh`（环境验证）、`clean-state-checklist.md`（收尾检查）、`docs/design/`（设计文档）、`docs/superpowers/plans/`（实现计划）。

---

## 0. 项目身份（30 秒了解）

- **是什么**：Golish 是一个开源的 **agentic 终端 + 渗透测试操作平台**，Tauri 2 桌面端。
- **栈**：Rust 2021（`backend/crates/` 下 50+ crate，workspace 模式）+ React 19 + TypeScript 6 + Vite 8 + Tailwind 4。
- **包管理**：`pnpm`（前端）+ `cargo`（后端，nextest 跑测试）。
- **统一接口**：所有命令走 `just`，详见 `justfile`。
- **持久化**：嵌入式 Postgres（`pg-embed` + `sqlx`），向量用 pgvector，知识图考虑 Graphiti。
- **LLM 编排**：`rig-core` 0.36 + 4 个 in-tree provider forks。
- **关键约束**：本项目是渗透测试平台，**安全与证据是第一公民**——任何阶段交付都必须能追溯到 evidence，不能只靠自然语言声称完成。

---

## 1. 开工流程（每轮新会话第一件事）

按顺序执行 1→2→3→4 才能开始动代码，**任何一步跳过都视为违反 harness 约定**：

1. **读上下文**
   - `agent-progress.md` → 看上一轮留下的状态、blocker、下一步建议
   - `feature_list.json` → 找当前 `in_progress` 的功能（**同一时间只能有一个**）；没有就从优先级最高的 `not_started` 里选
   - 当前会话用户的具体指令
   - `docs/modules/INDEX.md` → 模块地图入口；按本轮要动的模块找到对应「模块卡」，**动手前先读它**（职责 / 公开接口 / 依赖 / 坑 / 测试入口）。没有卡就先按现有模板补一张再动手

2. **验证基础环境**
   ```bash
   ./init.sh
   ```
   预期：依赖装好、`just check` + `just test-fe` + `just test-rust` 全绿。**如果失败，先修基础环境，不要在坏的地基上叠新功能**。

3. **如果接到新需求且不在 `feature_list.json` 中**
   - 简单改动（单文件 ≤3 处）：直接动手，结束时在 progress 里记一笔即可
   - 复杂改动（跨多 crate / 改 IPC / 改 schema / 涉及安全语义）：
     - 先在 `docs/design/YYYY-MM-DD-<name>.md` 写设计文档
     - 再在 `docs/superpowers/plans/YYYY-MM-DD-<name>.md` 写实现计划（按 `.cursor/skills/writing-plans/` 规范）
     - 再追加一条到 `feature_list.json`，状态设为 `not_started`
     - 然后才挑出来设为 `in_progress`

4. **选定一个功能后**
   - 把它在 `feature_list.json` 中标 `in_progress`
   - 在 `agent-progress.md` 新建一条"会话记录"，写明本轮目标

---

## 2. 工作中的规矩

### 2.1 一次只做一个功能

`feature_list.json` 同一时间**只能有一个 `in_progress`**。要切换功能必须先把当前的标回 `not_started` 或 `blocked` 并写明原因。

### 2.2 改 Rust 代码

- **动某 crate / 模块前**，先读 `docs/modules/backend/<crate>.md`（及相关子模块卡）了解职责与影响面；改完后按 §2.4 同步更新该卡
- 加新 Tauri command 必须按 `docs/development.md` 五步走：函数 → facade `pub use` → registry → 前端 wrapper → ts-rs 类型同步
- 命令命名 `<domain>_<verb>_<object>`（如 `ai_send_prompt`、`pentest_launch_tool`），禁止 camelCase 或动词在前
- **禁止**直接在 `backend/crates/golish/src/commands_registry.rs` 加 `use crate::foo::commands::*;` glob，必须走 `commands_facade/<domain>.rs`
- 改动后跑 `cd backend && cargo nextest run --status-level fail` 或 `just test-rust`
- clippy 必须零 warning：`just lint-rust`

### 2.3 改前端代码

- **动某前端子系统前**，先读 `docs/modules/frontend/<子系统>.md`；改完后按 §2.4 同步更新该卡
- 调 Tauri 走 `frontend/lib/api/<domain>.ts`，**禁止**裸 `invoke()`
- 跨 IPC 类型从 `frontend/lib/generated/`（由 ts-rs 生成）import，不要手写
- 三态 UI（loading / error / empty）每条异步路径都要画
- 改动后跑 `just check-fe`（biome + typecheck）+ `just test-fe`

### 2.4 改文档

- 设计变更 → `docs/design/YYYY-MM-DD-<topic>.md`（新文件，不覆盖旧设计）
- 实现计划 → `docs/superpowers/plans/YYYY-MM-DD-<topic>.md`
- 旧文档作废 → 在头部加 `> Superseded by <新文件>` 注释，不要直接删
- **模块卡**（agent-readable workspace 的 system-of-record）住在 `docs/modules/`：每个 crate 一张、每个目录子模块一张，入口是主索引 `docs/modules/INDEX.md`。**改了某模块的职责 / 公开接口 / 依赖关系，必须在同一次改动里更新它的卡 + 索引状态列**，让卡始终是「单一事实源」而非孤儿文档

### 2.5 涉及安全 / pentest 模块

- 任何 Recon / Vuln / Verify 阶段的产物必须能落进 evidence ledger，**不能只是自然语言总结**
- gate validator 是确定性规则，不要把"agent 自信说完成"当成 gate 通过
- 新增高风险扫描能力（active scan / exploit）必须先在 `docs/design/` 写授权与 scope 边界
- 详见 `docs/design/2026-05-20-agent-harness-strategy.md`、`docs/superpowers/plans/2026-05-20-golish-agent-harness-architecture.md`。注意：domain harness 当前 deferred，先补齐信息收集闭环与工具 evidence 契约。

### 2.6 提交前

```bash
just precommit   # = just check + just test
```

**全绿之前不允许 commit**。`just check` 跑 `fmt + check-fe + test-fe + lint-rust + test-rust-all`，是最严格的本地门禁。

### 2.7 高风险操作必须先问用户

以下动作**必须先在聊天里得到用户确认才能执行**：

- 删文件 / 删大量代码 / `git rm`
- 推送到远端、合并 PR、`git push --force`
- 改 DB schema / migration / `golish-db` crate
- 改已发布的 `ts-rs` 类型导致 frontend 类型链断
- 改 `release-please-config.json` / 版本号 / tag
- 执行任何对外部服务（API key、邮件、付费接口）发起的真实请求

### 2.8 严禁

- ❌ 用 `cat <<EOF`、`echo >`、`sed -i`、`awk` 编辑文件——必须用编辑工具
- ❌ 改代码不跑测试就声明"完成"
- ❌ 在 `.cursor/rules/` 之外创建新的 `.mdc`
- ❌ 把"已检查为空"和"未检查"混为一谈（pentest 数据模型核心约束）
- ❌ 直接修改 `frontend/lib/generated/` 下任何手写文件（由 ts-rs 生成）
- ❌ 在 transaction 里调外部 HTTP / MQ / 长耗操作

---

## 3. 完成定义（**整个 harness 最关键的部分，不要改这一节**）

一个功能能从 `in_progress` 切到 `passing`，**必须同时满足**：

1. **有验证命令实际跑过且证据被记录**
   - 跑的命令、退出码、关键输出片段，复制到 `agent-progress.md` 的"已记录证据"段
   - 跑的命令必须能在另一台机器上原样重放
2. **`feature_list.json` 对应条目的 `verification` 步骤逐条核对过**，并把通过证据填到 `evidence` 字段
3. **`just precommit` 全绿**（fmt + lint + 前后端 test 全过）
4. **没有引入未在本任务 scope 内的代码改动**——不要"顺手优化"无关代码
5. **下一轮会话不需要人工补救就能继续工作**——任何半成品状态必须写进 progress

满足 1+2+3+4+5 才能在 `feature_list.json` 把状态改为 `passing`。**任何一项缺失都必须停留在 `in_progress` 或转 `blocked`。**

```
没有新鲜的验证证据，不许宣称完成。
```

---

## 4. 收尾流程（每轮会话结束前必须做）

按顺序执行：

1. 跑一次 `just precommit`，确认全绿
2. 对照 `clean-state-checklist.md` 逐项核查
3. 更新 `agent-progress.md`：
   - 本轮目标 / 已完成 / 跑过的验证 / 已记录证据 / commit 记录 / 风险 / 下一步建议
4. 更新 `feature_list.json`：
   - 当前功能的 `status` 改对（`passing` / `blocked` / `in_progress` / `not_started`）
   - 如果是 `passing`，填 `evidence` 字段
   - 如果是 `blocked`，在 `notes` 写清楚阻塞原因和需要的输入
5. 如果本轮动过任何模块 → 更新对应 `docs/modules/` 卡片内容 + `docs/modules/INDEX.md` 状态列
6. 如果有未 commit 的半成品 → 在 progress 里明确写"以下文件已修改但未提交：..."

**未走完 1-6 不算"会话结束"**。

---

## 5. Golish 项目不变量（这些规则贯穿所有 PR，违反必须有 justification）

| # | 不变量 | 为什么 |
|---|---|---|
| I1 | 错误返回统一带 `code` 字段，前端按 map 翻译，不靠 HTTP status 做业务判断 | 全栈错误码契约 |
| I2 | 所有 CRUD 验资源所有权（IDOR），包括批量操作 | 渗透测试平台被打穿等于自身打脸 |
| I3 | 前端校验是 UX，后端必须独立做安全校验 | 攻击者会绕过前端 |
| I4 | Tauri command 命名 `<domain>_<verb>_<object>` | 防止命名碰撞（参见 `docs/development.md`） |
| I5 | 跨 IPC 类型用 `#[derive(ts_rs::TS)]` 同步到 frontend，不要手动维护两份 | 类型不同步是端到端 bug 的头号来源 |
| I6 | 设计变更走新 markdown 文件（不覆盖旧设计） | 保留决策历史 |
| I7 | 安全任务的阶段交付必须有 evidence，不能只是自然语言 | 内层 harness 核心 |
| I8 | "已检查为空" ≠ "未检查"——pentest deliverable 必须区分这两者 | gate validator 依赖 |
| I9 | 事务内不调外部 HTTP / MQ / 长耗操作 | 连接池雪崩 |
| I10 | 改 schema / migration 必须向后兼容再上服务（先扩字段、再上新代码、再清旧字段） | 回滚安全 |

---

## 6. 与已有体系的关系

- `.cursor/rules/global-enforcement.mdc`：定义铁律（先读后改、证据优先、修改与验证、高风险先确认、输出极简）和技能加载体系。AGENTS.md 是这些铁律在 Golish 项目语境下的具体化。
- `.cursor/skills/`：是 harness 的子模块（brainstorming / writing-plans / verification-before-completion / test-driven-development / systematic-debugging / executing-plans / tool-installation）。AGENTS.md 把它们串成完整 loop。
- `docs/superpowers/plans/`：当你接到需要分步实现的复杂任务时，**先**用 `writing-plans` skill 写一份计划放到这里，**再**用 `executing-plans` skill 按计划执行。
- 内层 domain harness（Rust 代码里跑的 stage gate / evidence ledger / Recon barrier）由 `docs/design/2026-05-20-agent-harness-strategy.md` 和 `docs/superpowers/plans/2026-05-20-golish-agent-harness-architecture.md` 跟踪；当前 deferred，等待信息收集闭环和工具 evidence 契约稳定。本 AGENTS.md 是**外层 meta harness**——约束的是开发 Golish 的 agent，不是 Golish 内部运行的 pentest agent。

---

## 7. 快速参考

```bash
./init.sh                    一键环境验证
just dev                     启动 Tauri 开发模式（端口 1420）
just dev-fe                  仅前端（mock Tauri 环境）
just check                   全套静态检查 + 单测
just test                    全部测试（前 + 后）
just test-fe                 仅前端测试（Vitest）
just test-rust               仅 Rust 测试（cargo nextest）
just test-e2e                Playwright E2E
just precommit               commit 前必跑（= check + test）
just kill                    清掉残留进程（占用 1420 端口时用）
```

文档入口：

| 想了解 | 看 |
|---|---|
| 项目整体架构 | `docs/architecture.md` |
| 上手开发 | `docs/development.md` |
| 一阶段 pentest 平台规划 | `docs/PHASE1_PENTEST_PLATFORM.md` |
| 内层 agent harness 架构 | `docs/design/2026-05-20-agent-harness-strategy.md`（deferred） |
| 内层 harness 实现计划 | `docs/superpowers/plans/2026-05-20-golish-agent-harness-architecture.md` |

---

## 8. 运行日志与产物位置（给后续 Cursor / agent 的指路）

> 本机跑 Golish 时，日志/产物落在**两个不同的根**，分析问题先认准位置：

- **进程级日志（全局，固定在 home）**：`~/.golish/`
  - `backend.log` — 后端 / agent / harness 全量 tracing（最常用；grep `harness::hook`、`gate BLOCK`、`Transcript writer initialized` 等）
  - `frontend.log` — 前端运行日志
  - `mcp-logs.log` — MCP
  - 旧日志轮转为 `backend.log.<ts>.bak`
- **AI 事件 transcript（按会话，JSONL，跟着 workspace 走）**：`{workspace}/.golish/transcripts/<session>/transcript.json`
  - `{workspace}` = 在 Golish 里打开的目录（**不是本开发仓库**）。例：当前测试用的 `/Users/christopherzheng/golish-platform/Test1`
  - 每会话一个目录；子 agent 在 `subagents/<agent_id>-<parent_req>/transcript.json`
  - 含 `harness_trace` 事件（gate PASS/BLOCK、evidence_booked、background notes 等）
  - 解析顺序见 `golish_events::op_trace::resolve_transcript_base`：`VT_TRANSCRIPT_DIR` 覆盖 > `{workspace}/.golish/transcripts` > `~/.golish/transcripts`

> **分析 Golish 运行问题时**：先定位 workspace（用户会给，或看 `~/.golish/backend.log` 里 `Transcript writer initialized ... at "<path>"` 那行），再读对应 `transcript.json`；全局 / 跨会话问题直接 grep `~/.golish/backend.log`。直接读这些文件即可分析，不依赖产品内的 `harness_trace` 工具 / `golish --replay`。

---

## 9. 给自己的最后一问（每次提交前默念）

1. 我跑过验证命令了吗？输出在哪里？
2. `feature_list.json` 和 `agent-progress.md` 状态对得上吗？
3. 我是不是在"顺手"改了不在 scope 内的东西？
4. 下一轮会话只看仓库内的文件，能继续推进吗？
5. 我宣称的"完成"，有证据支撑吗？

如果有任何一项答不上来，**回去补**，不要直接 commit。

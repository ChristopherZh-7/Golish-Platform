# clean-state-checklist.md

> **会话收尾必查清单**。关闭会话 / 切换 agent / 走人之前逐项检查。
>
> 任何一项不通过都视为"会话未结束"——必须修好或在 `agent-progress.md` 写清楚状态后才能离开。

---

## A. 代码与构建

- [ ] **如果本轮改动影响启动路径，已做定向启动验证**
  - 只在改动需要时验证 `just dev` 或 `just dev-fe`
  - 如果实际启动过，端口 1420 没有死进程占着（用 `just kill` 清理）

- [ ] **与本轮改动直接相关的定向验证全绿**
  - 跑过的命令和退出码已经记到 `agent-progress.md` 的"已记录证据"段
  - `./init.sh`、`just precommit` 和其他全量门禁仅在用户明确要求时运行；未授权时没跑不视为清单失败
  - 如果定向验证不足以支撑结论，已在 progress 记录剩余风险并请用户决定是否扩大验证

- [ ] **没有未提交的"半成品"游离在 working tree**
  - `git status` 显示的 modified / untracked 全部满足以下任一条件：
    - 已经 commit
    - 已经在 `agent-progress.md` 明确写出"以下文件已修改但未提交：..."并说明原因
    - 是本轮明确不要 commit 的临时文件（也要写进 progress）

- [ ] **没有引入新的 linter / typecheck / clippy 警告**
  - 不仅看是否报错，警告也算（`just lint-rust` 是 `-D warnings`）

---

## B. 文档与状态文件

- [ ] **`agent-progress.md` 已更新**
  - 在"会话记录"顶部插了本轮的一条
  - 7 个必填字段都写了：本轮目标 / 已完成 / 运行过的验证 / 已记录证据 / 提交记录 / 已知风险或未解决问题 / 下一步最佳动作
  - 没写"待补充"、"TODO 后续"等占位（这违反 `writing-plans` skill 禁止占位符的规矩）

- [ ] **`feature_list.json` 真实反映当前状态**
  - 本轮处理的功能 status 已经改对（`passing` / `blocked` / `in_progress` / `not_started`）
  - **没有假 `passing`**：所有 `passing` 都有 `evidence` 字段填好
  - 如果切到 `blocked`，`notes` 里写清楚阻塞原因和需要的输入
  - 同一时间只有一个 `in_progress`（或全为非 `in_progress`）

- [ ] **新文档放对位置**
  - 设计文档 → `docs/design/YYYY-MM-DD-<topic>.md`
  - 实现计划 → `docs/superpowers/plans/YYYY-MM-DD-<topic>.md`
  - 没有在项目根目录或随机位置丢临时文档

- [ ] **旧文档被 supersede 时有明确标注**
  - 例如 `> Superseded by docs/design/2026-05-20-xxx.md`
  - 没有"删了旧文档但没说为什么"的情况

---

## C. 安全与不变量（Golish 专属）

- [ ] **新增的 Tauri command 走完五步**（参考 `docs/development.md`）
  - 函数 → facade `pub use` → registry → 前端 wrapper → ts-rs 类型
  - 命名 `<domain>_<verb>_<object>`
  - 没有在 `frontend/lib/api/` 之外直接调 `invoke()`

- [ ] **新增 / 修改的 CRUD 都有资源所有权检查**（IDOR 防护）
  - `WHERE id = ? AND user_id = ?` 这类条件已加上
  - 批量操作也加了（不要只想着单条）

- [ ] **跨 IPC 的类型用 `#[derive(ts_rs::TS)]` 同步**
  - 没有手动维护两份前后端类型
  - 生成出来的 `frontend/lib/generated/` 已经 commit 在内

- [ ] **没有违反 Golish 不变量**（详见 `AGENTS.md §5`）
  - 错误返回带 `code` 字段
  - 后端有独立校验
  - 设计变更走新 markdown 不覆盖
  - pentest deliverable 区分 "已检查为空" 和 "未检查"
  - 事务内不调外部 HTTP / MQ
  - schema 改动向后兼容

- [ ] **没有把高风险动作偷偷做了**
  - 删文件 / 删大量代码 / 推到远端 / 改 migration / 改 release 配置 → 必须先在聊天里得到用户确认

---

## D. 交接准备度

- [ ] **下一轮会话不需要人工修复就能继续**
  - 与当前改动相关的定向验证可重放且仍然通过
  - `bash init.sh` 只在用户明确要求时运行，不是默认交接条件
  - 跑 `git status` 状态干净（或者所有非干净状态都已写进 progress）

- [ ] **下一步动作明确**
  - `agent-progress.md` 的"下一步最佳动作"具体到能直接照做
  - `feature_list.json` 的优先级排列合理，最高 priority 的就是下一轮应该挑的

- [ ] **没有"我记得 / 我刚才"型隐藏状态**
  - 所有关键决策、证据、风险都已落到文件
  - 即使 agent 重启或换人，仓库里的文件足以恢复完整上下文

---

## E. 最后一问（30 秒自查）

1. 我有没有跑过验证命令并把输出粘到 progress？
2. 我有没有"顺手"改了不在本轮 scope 内的代码？
3. 如果现在另一个 agent / 我自己明天接手，仅看仓库内文件，能继续推进吗？
4. 我宣称的"完成"，证据在哪？

任何一项答不上来 → 回去补，不要离开。

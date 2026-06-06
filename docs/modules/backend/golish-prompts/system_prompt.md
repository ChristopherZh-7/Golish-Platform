# golish-prompts / system_prompt

> **一句话职责**：system prompt **dispatcher**——按运行模式选模板：OpenAI provider → codex 风格；Task 模式（有 sub-agents）→ 多 agent 编排 prompt；Chat 模式 → 单 agent prompt。对调用方透明。

- **类型**：目录模块（属于 crate [`golish-prompts`](../golish-prompts.md)）
- **路径**：`backend/crates/golish-prompts/src/system_prompt/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 system prompt 模板分派逻辑（codex / task / chat 三态选择）时
- 改多 agent 团队委派段、specialist 路由表、单 agent 模式块时
- 改规则发现、agent-mode 指令、项目记忆文件读取等共享 helper 时

## 职责

`build_system_prompt*` 是 dispatcher：OpenAI providers 走 codex 风格（concise，reasoning 友好，不受 chat/task 影响）；`has_sub_agents==true` 走 task 模板（团队委派 + specialist 路由 + adviser 指导）；否则走 chat 模板（`<single_agent_mode>`，无 sub_agent 引用）。公开面不变，chat/task 拆分对调用方透明。

## 公开接口

| 符号 | 说明 |
|---|---|
| `build_system_prompt*` / `build_system_prompt_with_contributions` | system prompt 构建入口（dispatcher） |
| `get_agent_mode_instructions` / `read_project_instructions` | agent-mode 指令 / 项目记忆读取（共享 helper） |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | dispatcher + 共享 helper（规则发现/agent-mode/项目记忆） |
| `task.rs` / `chat.rs` | task 多 agent 模板 / chat 单 agent 模板 |
| `team_delegation.rs` / `instructions.rs` | 团队委派段 / agent-mode + 项目指令 |

## 依赖

- `golish-core`（`PromptContext`/`AgentMode`）、crate 内 `codex_prompt`（`build_codex_style_prompt`）/`prompt_registry`

## 注意事项 / 坑

- **三态分派是核心**：OpenAI→codex、task→多 agent、chat→单 agent；改分派条件要保证 chat 模式**不出现** `sub_agent_*` 引用（单 agent 纯净）。
- 公开面对调用方稳定（拆分透明）；改内部模板别破坏对外签名。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-prompts system_prompt
```

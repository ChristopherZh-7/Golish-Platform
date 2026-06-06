# golish-agent-kit / system_hooks

> **一句话职责**：可扩展 hook 系统——在 agent 执行中注入上下文消息：消息 hook（用户输入/agent 输出关键词或正则匹配）+ 工具 hook（pre/post tool，可 block 或注入消息）。

- **类型**：目录模块（属于 crate [`golish-agent-kit`](../golish-agent-kit.md)）
- **路径**：`backend/crates/golish-agent-kit/src/system_hooks/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 加/改 hook（按用户消息/agent 响应/pre-tool/post-tool 触发，注入上下文或拦截工具）时
- 改 hook matcher（Keyword/Regex/ToolName/ToolCategory/自定义谓词）时

## 职责

提供在 agent 执行各点注入上下文消息的机制。消息 hook 触发于 UserMessage / AgentResponse（Keyword/Regex/自定义匹配）；工具 hook 触发于 PreToolExecution（可 block 或注入）/ PostToolExecution（注入）。

## 公开接口

| 符号 | 说明 |
|---|---|
| hook registry + 注册 API | 注册/触发 hook |
| 消息 hook（UserMessage / AgentResponse） | 关键词/正则匹配注入 |
| 工具 hook（Pre/PostToolExecution） | 工具前后注入/拦截 |
| matcher（Keyword/Regex/ToolName/ToolCategory/Custom） | 触发条件 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | hook 系统注册 + 触发 + matcher |

## 依赖

- crate 内；`regex`（正则 matcher）

## 注意事项 / 坑

- PreToolExecution hook **可 block 工具**——这是安全/策略注入点，改要小心别误拦合法工具。
- 与 `tool_policy`/`hitl` 是不同机制（hook 是上下文注入/拦截，policy 是声明式访问控制）。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-kit system_hooks
```

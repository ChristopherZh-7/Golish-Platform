# golish-sub-agents

> **一句话职责**：sub-agent 系统——sub-agent 定义（自定义 system prompt + 工具限制）、registry、发现/加载（YAML frontmatter）、执行器（含 udiff 应用）、prompt registry/contributor，以及默认 sub-agent 集。

- **类型**：crate（Layer 2 · agent 基础设施）
- **路径**：`backend/crates/golish-sub-agents/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 sub-agent 定义/registry、`execute_sub_agent` 执行链、嵌套深度（`MAX_AGENT_DEPTH`）时
- 改 agent 文件加载（YAML frontmatter + 文件系统发现）、默认 sub-agent 集时
- 改 sub-agent prompt 模板（tera）/ contributor / skills 注入时

## 职责

提供 sub-agent 编排基础设施：定义专门化 sub-agent、管理可用 agent 注册表、在 agent 间传递 context、带工具支持地执行 sub-agent。通过 `ToolProvider` trait 注入工具定义/执行，避免对上层 agent runtime 的反向依赖。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `SubAgentDefinition` / `SubAgentRegistry` | sub-agent 定义与注册表 |
| `SubAgentContext` / `SubAgentResult` / `AgentSource` / `MAX_AGENT_DEPTH` | 上下文/结果/来源/深度上限 |
| `execute_sub_agent` / `SubAgentExecutorContext` / `ToolProvider` | 主执行函数 + 工具注入 trait |
| `create_default_sub_agents` | 默认 sub-agent 集 |
| `discover_agents` / `AgentFileInfo` | 文件系统发现 + 加载 |
| `PromptRegistry` / `PromptContext` / `SubAgentPromptContributor` | prompt 注册/上下文/贡献者 |
| `StageToolGuard` / `StageToolHider` / `SubAgentToolRouter` / `SubAgentToolResultHook` / `SubAgentToolObserver` / `PostShellHook` / `SubAgentChainPersistence` | 阶段工具守卫/路由/工具结果后处理/telemetry observer/持久化（executor_types） |
| `SubmitRepairMode` / `SubmitRepairKind` | StageRefiner 产出的 repair directive 在 executor 内的兼容投影；负责 resume repair lock 与 allowed/forbidden tools |

## 依赖

- **内部**：`golish-core`、`golish-udiff`、`golish-tools`、`golish-shell-exec`、`golish-llm-providers`、`golish-json-repair`、`golish-skills`
- **外部**：`rig-core`、`serde_yaml`、`tera`、`dirs`

## 被谁依赖 / 改动影响面

`golish-agent-kit`、`golish-agent-runtime`、`golish-agent-bridge`、`golish-agent-app`、`golish`。整条 agent 栈都依赖它。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `definition/` | 定义/registry/context/result | [→](golish-sub-agents/definition.md) |
| `executor/` | 执行链（execute_sub_agent） | [→](golish-sub-agents/executor.md) |
| `executor_helpers/` | 执行辅助（content/history/helper） | [→](golish-sub-agents/executor_helpers.md) |
| `defaults/` | 默认 sub-agent 集 + prompt fallback | [→](golish-sub-agents/defaults.md) |

## 关键文件

`discovery.rs`、`file_loader.rs`、`prompt_registry.rs`、`prompt_contributor.rs`、`schemas.rs`、`transcript.rs`、`executor_types.rs`、`executor_udiff.rs`。

## 注意事项 / 坑

- `MAX_AGENT_DEPTH` 限制嵌套递归——改 sub-agent 调 sub-agent 时务必尊重深度上限，防失控。
- 工具走 `ToolProvider` trait 注入（非直接依赖上层 runtime），保持本 crate 处于 L2，不要引入向上依赖。
- 默认 `recon` 子 agent 是 `target_intel` 的 provider-only 生产者：不暴露 `list_in_scope_targets` / `pentest_run`，避免在 intel 阶段查询尚未生产的目标或 fallback 到 subfinder/dig 类扫描路径；`prober` 消费 ranked attack-surface seeds，`enumerator` 必须先消费 `stage_worklist_status` / `stage_worklist_next` 的 DB-truth stage-local worklist，再把 `list_enumeration_web_roots` 当 web-root 上下文。`enumerator` 不暴露 `manage_targets` / `record_finding`，因为 `enumeration` 是 content coverage 阶段，产物是 DB truth + claims + non-found terminal coverage，不是资产状态更新或漏洞 findings。
- sub-agent 的 registry/router 工具执行会在 `with_agent_session`、`with_agent_tool_context` 和 `with_agent_tool_output_sender` 下运行；因此 Enumerator/Prober 里调用的 direct bridge tools 可以把真实运行过程发成 `tool_output_chunk`，在子 agent 详情 Output 区实时可见。新增工具执行分支时不能丢这三个 scope。
- `browser_collect_js_api` / `js_extract_apis` / `route_probe_paths` 这类长耗时 direct bridge tools 不走 sub-agent 外层 `tokio::time::timeout`，否则 future 会被 drop，工具来不及写最终 DB truth。shell/pentest 命令的软超时转后台由上层 `golish-app-core::background_jobs` 处理；若要让 direct bridge tools 也返回 `backgrounded/job_id`，需要经 `ToolProvider` 注入后台适配 seam，不能在本 L2 crate 直接依赖 app-core。
- `SubAgentToolResultHook` 只提供通用结果后处理注入点；具体 harness/evidence/source_query 副作用由上层 runtime 注入，避免本 crate 反向依赖 DB/harness。
- `SubAgentToolObserver` 是上层 runtime 的工具观察点；当前 Mentor 只做 telemetry，不再注入模型可见纠错。真正的 repair guidance 由 runtime/agent-kit 的 StageRefiner 产出，再通过 `SubmitRepairMode` 注入 executor。
- sub-agent 工具结果要区分两条通道：`AiEvent::SubAgentToolResult` / transcript 保留 raw JSON 供 UI 和证据追溯；回灌给下一轮 LLM 的 `ToolResult` 会按工具做 model-visible compaction。`route_probe_paths` / `list_enumeration_web_roots` / JS collect/extract 这类大结果只给 counts、samples、next_action 和 raw-retained 标记，避免 stage_run worker 把完整 route/error/coverage 数组反复塞进上下文。
- `SubmitRepairMode` 的 coverage-gap repair 不再按“批量”本身拦截 `pentest_run`；EAS gap 很多且 gate 给出非空 `coverage_gap_actions` 时，必须允许 `input_lines` / list-file 批量探测。enumeration gap repair 会按 `GOLISH-ENUM-*` action 自动放行 direct tools（`browser_collect_js_api` / `js_collect` / `js_extract_apis` / `route_probe_paths`），但会校验 `target_url`/`base_url` 必须落在 `coverage_gap_actions` 点名资产内。DIR gap 只能用 `route_probe_paths`，不允许回退 ffuf/gobuster/feroxbuster；PARAM gap 默认用 browser/js_extract/crawler 已观察到的请求、query、form 与 `param_hints` 补 `api_endpoints.params`，bounded crawler CLI（如 katana）只能经 `pentest_run` 作为 URL 来源。repair 模式必须继续允许 `check_stage_asset_coverage`，因为它是 submit 前自检和修复收口的只读工具，不能被 repair lock 挡掉。它仍会阻止 CIDR/range sweep、隐藏 list file（未提供可校验的 `input_lines`/`stdin`）以及任何不在 `coverage_gap_actions` 中的目标，确保批量只覆盖 deterministic gate 点名的资产。若 coverage needs_fix 没有结构化 `coverage_gap_actions`，repair 只允许 coverage/DB 查询、后台 job 控制和 resubmit，不能启动 `pentest_run` 或 guessed-domain probes。
- doc 注释提到的 `golish-web` / `vtcode-core` 为历史描述；当前 Cargo.toml 实际内部依赖以本卡「依赖」段为准。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-sub-agents
```

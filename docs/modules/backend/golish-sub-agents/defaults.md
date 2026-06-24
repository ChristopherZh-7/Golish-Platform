# golish-sub-agents / defaults

> **一句话职责**：默认 sub-agent 定义——`create_default_sub_agents`(_from_registry) 装配预配置 sub-agent；`prompts` 持硬编码 `build_*_prompt` + `WORKER_PROMPT_TEMPLATE`（作为模板驱动 registry 版本的 fallback）。

- **类型**：目录模块（属于 crate [`golish-sub-agents`](../golish-sub-agents.md)）
- **路径**：`backend/crates/golish-sub-agents/src/defaults/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改默认 sub-agent 集（worker 等）的定义或硬编码 prompt fallback 时
- 改模板驱动版本（`prompts/*.tera` + DB override）与硬编码 fallback 的关系时

## 职责

`builder` 公开构造器 `create_default_sub_agents` / `create_default_sub_agents_from_registry` 装配 `SubAgentDefinition`；`prompts` 持每个硬编码 `build_*_prompt` + `WORKER_PROMPT_TEMPLATE` 常量，作为模板驱动 registry 版本（优先 `prompts/*.tera` + DB override）的 fallback。

## 公开接口

| 符号 | 说明 |
|---|---|
| `create_default_sub_agents` / `create_default_sub_agents_from_registry` | 默认集构造器 |
| `WORKER_PROMPT_TEMPLATE` | worker prompt 模板常量 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | re-export builder + 常量 |
| `builder/` | `SubAgentDefinition` 装配 |
| `prompts/` | 硬编码 `build_*_prompt` + 模板常量 |

## 依赖

- crate 内 `definition::SubAgentDefinition`、`prompt_registry`；tera（模板）

## 注意事项 / 坑

- 硬编码 prompts 是 **fallback**：registry 优先用 `prompts/*.tera` + DB override；改默认行为先确认走的是哪条路径。
- `from_registry` 版本会合并 DB/模板 override；纯 `create_default_sub_agents` 是无 registry 的基线。
- `prober` prompt 是 `external_attack_surface` 的 active worker contract：优先 `list_attack_surface_seeds`，按 domain/ip/url/cidr 类型决定 liveness/port/service 动作；显式 coverage 要填 evidence 与 denominator（尤其 SERVICE-FINGERPRINT 的 `tested_units/total_units = 已指纹开放端口/发现开放端口`），不能把 HTTP liveness 当作 PORT/SERVICE 证明。
- prober / enumerator prompt 的后台任务约束要强调：慢扫描只跑一次，等 completion note 和 submit barrier 落证据后再提交；不要在后台 job 未完成时反复 submit 或重跑同一命令。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-sub-agents defaults
```

# golish-recon-app / agent_tools

> **一句话职责**：让 harness `target_intel` 阶段由 AI 直接驱动被动资产情报的 agent 工具——包 `asset_intel::run_passive_intel`：`recon_discover_subsidiaries`（ENScan 子公司发现）+ `recon_map_assets`（0.zone/quake 等 provider survey）+ `recon_lookup_whois`，结果落 evidence ledger 供阶段引用。

- **类型**：目录模块（属于 crate [`golish-recon-app`](../golish-recon-app.md)）
- **路径**：`backend/crates/golish-recon-app/src/agent_tools/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 AI 驱动的被动资产情报工具（子公司发现 / 资产富化）、其 IDOR 守卫、evidence 落账时

## 职责

把被动 asset-intel 引擎包成 AI 工具，让 harness 的 `target_intel` 阶段由 AI（而非 GUI 按钮）执行。这些工具都取确认的 engagement `organization_id`（scoping 期 org-first 创建）且 project-scoped（IDOR 守卫 I2），结果是 JSON 摘要并 book 进 evidence ledger，让 coverage cell 能引用真实 evidence id。设计见 `2026-06-06-intel-stage-ai-driven-per-mode §3.3`。

## 公开接口

| 符号 | 说明 |
|---|---|
| `recon_discover_subsidiaries`（Tool） | ENScan 子公司发现 |
| `recon_map_assets`（Tool） | 0.zone/quake/… provider survey + DB landing |
| `recon_lookup_whois`（Tool） | RDAP WHOIS，写 `organizations.whois` |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | 两个 AI 工具（包 `asset_intel::run_passive_intel`） |

## 依赖

- crate 内 `asset_intel`；`golish-core::Tool`、`sqlx`、`uuid`

## 注意事项 / 坑

- **不变量 I2**：取 `organization_id` + project-scoped——工具绝不能碰别 project 的 org。
- **不变量 I7**：结果必须 book 进 evidence ledger（阶段 coverage 要引真实 evidence id）。
- `run_passive_intel` 的 JSON summary 带 `providerStatus`，runtime 用它写 `source_query_log`；这证明 provider/source terminal，不等于全网完整性。
- `recon_map_assets(organization_id=...)` 的普通 org/company survey 会由 `asset_intel::run_passive_intel` 自动扩展 bounded owned apex domains；summary 可能带 `domainExpansions[]`，runtime 会把其中 nested `providerStatus` 写成 `source_query_log(target=<apex>)`。工具 schema 里的 `domain` 参数保留给 targeted repair/manual supplement，不是默认流程。
- 与 GUI 路径（`integrations`/`organization_recon`）共用底层 `asset_intel`，行为应一致。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-recon-app agent_tools
```

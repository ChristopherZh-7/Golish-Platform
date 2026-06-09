# golish-recon-app / asset_intel

> **一句话职责**：Discover Assets 的资产情报服务——provider-agnostic：workspace 要候选、provider 返归一化记录、只有被批准的候选后续才成 scope；含 ENScan 子进程编排，输出写 `golish-projects` 文件存储。

- **类型**：目录模块（属于 crate [`golish-recon-app`](../golish-recon-app.md)）
- **路径**：`backend/crates/golish-recon-app/src/asset_intel/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改资产发现（候选采集、provider 归一、ENScan 子进程、候选→scope 流程）时
- 改 asset intel 的 runtime/service 子层或输出落盘时

## 职责

Phase 1 provider-agnostic 资产情报：`run_passive_intel` 等调被动 provider 取候选，归一成记录，写入 `OrganizationCandidates`（经 `organizations`），输出落 `golish-projects` 文件存储。候选需用户批准才进 scope。

## 公开接口

| 符号 | 说明 |
|---|---|
| `run_passive_intel`（service/runtime） | 被动情报采集入口 |
| 候选/归一记录类型 | provider-agnostic 记录 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | 服务编排 + 候选写入 |
| `service/` / `runtime/` | 服务层 / 运行时（子进程/取消） |
| `runtime/{cli,http,native}.rs` | 三种 provider runtime kind：`cli_json`（ENScan）/`http_json`（0.zone/quake）/`native_provider`（fofa/hunter/shodan，桥接 `golish-intel-providers` 注册表） |

## 依赖

- crate 内 `organizations`（候选写）、`golish-pentest::models::ToolConfig`（ENScan 等）、`golish-projects`（输出落盘）、`golish-core`（事件）

## 注意事项 / 坑

- **provider-agnostic**：候选先归一、经用户批准才成 scope——别让发现直接写 scope（绕过审批）。
- ENScan 经子进程（`tokio::process`）；输出落 projects 文件存储（非 DB 大字段）。
- 被 `agent_tools`（harness target_intel 阶段）包装成 AI 工具调用。
- **三个 runtime kind**（`AssetIntelRuntimeConfig`）：`cli_json` / `http_json` / `native_provider`。新加测绘 provider 若 `golish-intel-providers` 注册表已有实现（fofa/hunter/shodan/0.zone/quake），优先写 `native_provider` toolsconfig（复用原生鉴权/编码/字段映射），别在 toolsconfig 用 http_json 重写 API。
- `native_provider` 凭据走 `read_vault_secret`（与 http_json 同款，含 legacy `name=tool_id` 回退）；无 key → `Unavailable`（不伪造，I8），`provider_output_is_trusted` 仅信 `Completed`/`CheckedEmpty`。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-recon-app asset_intel
```

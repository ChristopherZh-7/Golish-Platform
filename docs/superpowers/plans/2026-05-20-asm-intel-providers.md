# ASM 多 Provider 集成 · 实施计划

> 日期：2026-05-20
> 关联设计文档：`docs/design/2026-05-20-asm-intel-providers.md`
> 分支：`feat/asm-intel-providers`
> 预计总工时：2-3 天

按 `superpowers/executing-plans` 规范，分 4 个 Phase，每个 Phase 含 review checkpoint。

---

## Phase 1 · 架构骨架（0.5-1 天）

**目标**：建立 `golish-intel-providers` crate + design doc + plan + feature_list 更新。

**Tasks**：

| # | 任务 | 范围 | 验收 |
|---|---|---|---|
| T1.1 | 写 design doc | `docs/design/2026-05-20-asm-intel-providers.md` | 文件存在 + 含 §0-§9 |
| T1.2 | 写 plan（本文件）| `docs/superpowers/plans/2026-05-20-asm-intel-providers.md` | 文件存在 + 含 Phase 1-4 |
| T1.3 | `feature_list.json` 加新条目 `asm-intel-providers` | feature_list.json | priority 设定 + status="in_progress" |
| T1.4 | 创建新 crate `golish-intel-providers` 骨架 | backend/crates/golish-intel-providers/{Cargo.toml,src/lib.rs} | `cargo check -p golish-intel-providers` 通过 |
| T1.5 | 定义 `IntelProvider` trait + `IntelError` + `ProviderRecord` | src/lib.rs + src/error.rs + src/types.rs | 单测：trait 对象可装箱 |
| T1.6 | `shared::api_key`（从 vault 读 key） | src/shared/api_key.rs | 单测：mock vault 返回 key |
| T1.7 | `shared::rate_limit` | src/shared/rate_limit.rs | 单测：2req/s 限速正确 |
| T1.8 | 更新 backend workspace Cargo.toml | backend/Cargo.toml | 含新 crate 在 members + workspace.dependencies |

**Review Checkpoint**：用户审 design doc 关键决策 + 看 crate 骨架文件结构是否合理。

---

## Phase 2 · 0.zone 首个 Provider 落地（0.5-1 天）

**目标**：完整实现 0.zone 7 query_type，端到端跑通 HTTP→ProviderRecord→DB。

**Tasks**：

| # | 任务 | 范围 | 验收 |
|---|---|---|---|
| T2.1 | `zone/client.rs` HTTP 客户端 | POST /api/data/ 含 key | mock test 2 个 case（200/401）|
| T2.2 | `zone/types.rs` 7 query_type 响应结构 | site/domain/email/apk/sensitive/code/member | serde round-trip 单测 |
| T2.3 | `zone/mapper.rs` 响应 → ProviderRecord | 按 query_type 提取 fields key | 7 个 mapper 单测 |
| T2.4 | `zone/mod.rs` 实现 `IntelProvider` trait | impl + register | 集成测试 |
| T2.5 | `output_store/organizations.rs` 新增 `store_organization_update` writer | 按 fields key 路由到列 | nextest 覆盖 5 个字段 |
| T2.6 | `output_store/mod.rs` match 加 `organization_update` 分支 | dispatch 路由 | nextest 全绿 |
| T2.7 | `output_store/store_trait.rs` + `pg_adapter.rs` 加 trait method | OutputStore::store_organization_update | 编译通过 |
| T2.8 | `resources/toolsconfig/0-zone.json` | 含 8 个 skill | tool scanner 识别 + UI 显示 |

**Review Checkpoint**：手动跑一次 `cargo nextest run -p golish-intel-providers --package golish-pentest` 全绿。

---

## Phase 3 · IPC + 前端 Settings（0.5 天）

**目标**：Settings UI 配 key、调用 ASM、看到 organizations 变更。

**Tasks**：

| # | 任务 | 范围 | 验收 |
|---|---|---|---|
| T3.1 | `golish/src/tools/intel_providers/mod.rs` + commands.rs | intel_query / intel_list_providers / intel_test_connection | cargo check |
| T3.2 | `golish/src/commands_facade/intel_providers.rs` | `pub use` 暴露 commands | `just check-rs` |
| T3.3 | `commands_registry.rs` 注册新命令 | 加 3 个 Tauri command | tauri runtime 识别 |
| T3.4 | ts-rs 同步类型 | frontend/lib/generated/ | `just check-fe` |
| T3.5 | `frontend/lib/api/intel.ts` IPC wrapper | invoke 3 个命令 | TS 编译过 |
| T3.6 | `frontend/components/Settings/IntelProvidersSettings/index.tsx` | 主面板 + provider 列表 | UI 渲染 |
| T3.7 | `IntelProvidersSettings/ProviderCard.tsx` | 单 provider 卡（含 KeyEditor 嵌入） | UI 交互 |
| T3.8 | `IntelProvidersSettings/KeyEditor.tsx` | 接 vault.entry CRUD | UI 表单 |
| T3.9 | `SettingsTabContent.tsx` 新增 NAV_ITEMS 一条 "intel" | nav 显示 | UI 跳转 |
| T3.10 | i18n 翻译键 | settings.intelProviders.* | 中英文 |

**Review Checkpoint**：前端 dev server 启动 + Settings 能看到 Intel Providers 入口 + 能输入 key 保存。

---

## Phase 4 · E2E + 后续 Provider 占位（0.5 天）

**目标**：端到端验证 + 给 fofa/quake/hunter/shodan 预留目录。

**Tasks**：

| # | 任务 | 范围 | 验收 |
|---|---|---|---|
| T4.1 | Playwright E2E spec | tests/e2e/intel-providers.spec.ts | `just test-e2e` 全绿 |
| T4.2 | `fofa/` 占位（stub + TODO 注释）| src/fofa/{mod,client,types,mapper}.rs | `cargo check` |
| T4.3 | `quake/` 占位 | 同上 | 同上 |
| T4.4 | `hunter/` 占位 | 同上 | 同上 |
| T4.5 | `shodan/` 占位 | 同上 | 同上 |
| T4.6 | 更新 `agent-progress.md` 本轮记录 | session log | review |
| T4.7 | `feature_list.json` 状态切 `passing` | json diff | review |
| T4.8 | `just precommit` 全绿 | 全套检查 | 0 error 0 warning |

**Review Checkpoint**：用户确认 E2E + 验收清单全过 → 可以 merge。

---

## 任务依赖图

```
T1.1 → T1.2 → T1.3 → T1.4 → T1.5 ─┐
                                   ├→ T1.6 → T1.7 → T1.8 → Phase1 done
                                   │
Phase1 → T2.1 → T2.2 → T2.3 → T2.4 ─┐
                                    ├→ T2.5 → T2.6 → T2.7 → T2.8 → Phase2 done
                                    │
Phase2 → T3.1 → T3.2 → T3.3 → T3.4 ─┐
                                    ├→ T3.5 → T3.6 → T3.7 → T3.8 → T3.9 → T3.10 → Phase3 done
                                    │
Phase3 → T4.1 → T4.2~T4.5（并行）→ T4.6 → T4.7 → T4.8 → DONE
```

---

## 风险与缓解

| 风险 | 概率 | 缓解 |
|---|---|---|
| 0.zone API 实际响应字段与 GitHub 上 wrapper 不一致 | 中 | mapper 用 serde_json::Value 兜底 + raw 字段保留原始响应 |
| vault 读取 key 异步性 | 低 | tokio mutex 内部串行化 |
| Tauri command 注册漏一个导致 IPC 404 | 中 | 加单测：枚举命令清单 |
| 前端 NAV_ITEMS 顺序影响 UX | 低 | 放在 "providers" 之后（AI providers 旁边）|
| feature_list.json 同一时间多个 in_progress | 中 | 设计文档明确说明，本轮在 notes 里标"前 3 个游离 in_progress 是历史，本条目是真正在做的" |

---

## 不在本计划范围

- credentials 表 + credential_add writer（baseline §9.6）
- api_endpoint_add db_action（baseline §9.4）
- targets 表 9 个 recon 字段 db_action 修复（baseline §9.1）
- agent loop 调用 ASM provider 自动跑（Phase 5+）

这些都是独立 PR，本 plan 专注 ASM provider 抽象 + 0.zone 首发 + key 管理 UI。

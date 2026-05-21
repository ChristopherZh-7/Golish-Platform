# Integrations 集成中心 · 实施计划

> 日期：2026-05-21
> 关联设计文档：`docs/design/2026-05-21-integrations.md`
> 分支：跟随 `asm-intel-providers`（不另开新分支）
> 预计总工时：1.5-2 天

按 `superpowers/executing-plans` 规范分 5 个 Phase，每个 Phase 含 review checkpoint。**严格按顺序执行，每个 Phase 结束必须 review 通过才能进下一个**。

---

## Phase 1 · 新 crate 骨架 + Schema 类型定义（3-4 小时）

**目标**：建好 `golish-integrations` crate；定义所有跨层共享类型（schema / storage / health / field）；ts-rs 同步到前端。

**Tasks**：

| # | 任务 | 范围 | 验收 |
|---|---|---|---|
| T1.1 | 新建 crate 骨架 | `backend/crates/golish-integrations/{Cargo.toml,src/lib.rs}` | `cargo check -p golish-integrations` 通过 |
| T1.2 | 定义 `IntegrationSchema` / `IntegrationGroup` / `Field` / `Storage` / `TestKind` | `src/schema.rs` | serde round-trip 单测 + `#[derive(ts_rs::TS)]` |
| T1.3 | 定义 `IntegrationHealth` / `FieldValue` / `IntegrationError` | `src/types.rs` + `src/error.rs` | 单测覆盖 enum 序列化 |
| T1.4 | 定义 `StorageBackend` trait + `SchemaResolver` trait | `src/traits.rs` | 单测：trait object 可装箱 |
| T1.5 | 更新 backend workspace `Cargo.toml` | `backend/Cargo.toml` | 含新 crate 在 members + workspace.dependencies |
| T1.6 | tool JSON schema 扩展 | `backend/crates/golish-pentest-domain/src/.../tool_config.rs` 加 `integration: Option<IntegrationSchema>` 字段 | `cargo nextest run -p golish-pentest-domain` 全绿 |
| T1.7 | ProviderMeta 加 `integration_schema` 字段 | `backend/crates/golish-intel-providers/src/types.rs` | 编译过；5 个 provider 默认 None |

**Review Checkpoint**：用户审 schema 类型签名（关键决策点：FieldType 枚举是否够用 / Storage 三态的字段是否好序列化 / TestKind 是否覆盖到 ENScan 的 exec 测试）。

---

## Phase 2 · Storage Backends（4-5 小时）

**目标**：3 个 backend 全部能 read/write/clear/cleartext，单测覆盖。

**Tasks**：

| # | 任务 | 范围 | 验收 |
|---|---|---|---|
| T2.1 | `VaultBackend` 实现 | `src/storage/vault.rs` | 5 个单测：write 多行 / read 命中 / read alias 老格式 / clear 精确删除 / cleartext 解码 |
| T2.2 | `ExternalFileBackend` YAML 渲染 | `src/storage/external_file.rs` + serde_yaml | 4 个单测：写新文件 / preserve_unknown_keys 合并 / atomic write / backup 滚动 3 份 |
| T2.3 | `ExternalFileBackend` JSON 渲染 | 同上文件 | 2 个单测：写 / 合并 |
| T2.4 | `SettingsBackend` 实现 | `src/storage/settings.rs` | 2 个单测：通过 SettingsManager 读 / 写 `network.github_token` 路径 |
| T2.5 | `SchemaResolver` 实现 | `src/resolver.rs` | 单测：从 fixture toolsconfig 目录扫出 1 个 integration schema + ProviderMeta 5 个 |
| T2.6 | `Tester` 实现（exec / http / builtin 三种） | `src/tester.rs` | 3 个单测：exec mock command match ok_regex；http mock server 200 OK；builtin 转发到 IntelProvider |

**Review Checkpoint**：用户人工验证 ExternalFileBackend 写入 `~/.config/enscan/config.yaml` 真能让 ENScan 读到（用一个 sandbox cookie）。

---

## Phase 3 · IPC 命令（2 小时）

**目标**：5 个 Tauri command 全部注册，前端能 invoke。

**Tasks**：

| # | 任务 | 范围 | 验收 |
|---|---|---|---|
| T3.1 | `integrations_list_schemas` | `golish/src/tools/integrations/commands.rs` | 单测：返回 ENScan + 5 provider |
| T3.2 | `integrations_get` | 同上 | 单测：vault 数据返回 has_value=true |
| T3.3 | `integrations_set` | 同上 | 单测：写 vault；写 external_file 路径变化 |
| T3.4 | `integrations_clear` | 同上 | 单测：3 行全删 |
| T3.5 | `integrations_test` | 同上 | 单测：mock builtin/exec/http 三种 path |
| T3.6 | `commands_facade/integrations.rs` 暴露 | `golish/src/commands_facade/integrations.rs` | `pub use crate::tools::integrations::*;` |
| T3.7 | `commands_registry.rs` 注册 5 个新命令 | 同名文件 | tauri runtime 识别 |
| T3.8 | ts-rs 同步类型 | `frontend/lib/generated/` 自动生成 | `just check-fe` 通过 |
| T3.9 | `frontend/lib/api/integrations.ts` IPC wrapper | 新建 | TS 编译过 |

**Review Checkpoint**：在 Tauri devtools 用 `__TAURI__.invoke('integrations_list_schemas')` 能返回结构（schema 至少含 enscan-go / 0.zone / github 三项）。

---

## Phase 4 · 前端 Integrations UI（4-5 小时）

**目标**：动态表单组件库 + Settings → Integrations 页 + 三态完备。

**Tasks**：

| # | 任务 | 范围 | 验收 |
|---|---|---|---|
| T4.1 | `<SecretInput>` / `<SecretTextarea>` | `frontend/components/Settings/IntegrationsSettings/fields/` | reveal 切换、30s 自动遮回；测试组件 |
| T4.2 | `<UrlInput>` / `<SelectField>` / `<BooleanField>` / `<ProxyInput>` | 同上 | 表单 controlled，受控 UX |
| T4.3 | `<IntegrationGroup>` 按 fields[] 动态渲染 | `IntegrationsSettings/IntegrationGroup.tsx` | 单测：3 字段 schema 渲染 3 个 input |
| T4.4 | `<IntegrationCard>` 折叠卡 + status badge | `IntegrationsSettings/IntegrationCard.tsx` | 单测：未配置 / 已配置 / 已过期 三态 badge |
| T4.5 | `<TestButton>` + `<IntegrationHealth>` 渲染 | `IntegrationsSettings/TestButton.tsx` | 5 种 health 状态 mapping |
| T4.6 | `<CategoryNav>` + 搜索框 | `IntegrationsSettings/CategoryNav.tsx` | 按 category 过滤 + fuzzy match |
| T4.7 | `IntegrationsSettings/index.tsx` 入口组合 | 同上 | 三态：loading skeleton / error banner / empty state |
| T4.8 | i18n 翻译键 | `frontend/lib/i18n/en.json` + `zh-CN.json` 新增 `integrations.*` | 中英文齐全 |

**Review Checkpoint**：UI 看截图确认布局 + 交互（不接真实数据也能 storybook 跑）。

---

## Phase 5 · 接入 + 迁移 + 删除老 UI（3-4 小时）

**目标**：ENScan_GO + 5 intel provider + GitHub Token 全接入；删 IntelProvidersSettings；端到端验证。

**Tasks**：

| # | 任务 | 范围 | 验收 |
|---|---|---|---|
| T5.1 | ENScan_GO 加 `integration` 段 | `resources/toolsconfig/enscan-go.json` | 5 groups（aqc/tyc/kc/rb/miit）；TYC 3 字段；UI 可渲染 |
| T5.2 | 5 个 intel provider 填 `integration_schema` | `golish-intel-providers/src/{zone,fofa,quake,hunter,shodan}/mod.rs` | 编译过；UI 可渲染 |
| T5.3 | GitHub Token 加 `core_integrations.json` | `backend/crates/golish/resources/core_integrations.json` 或类似 | UI 渲染；test 用 GitHub API /user |
| T5.4 | `SettingsTabContent.tsx` 改 nav: `intel-providers` → `integrations` | 单文件 | nav 可点击进入新页 |
| T5.5 | 删除 `frontend/components/Settings/IntelProvidersSettings/` | 整目录 + tsconfig path 清理 | grep IntelProvidersSettings 找不到 |
| T5.6 | Network tab 移除 GitHub Token 输入框 | `frontend/components/Settings/NetworkSettings.tsx` | 仅保留 proxy_url；i18n 提示用户去 Integrations |
| T5.7 | 老 vault entry read alias 端到端测试 | 手工：用旧 0.zone key 配 → 走新 UI 验证 | UI 显示已配置；test_connection 成功 |
| T5.8 | E2E Playwright | `tests/e2e/integrations.spec.ts` | 至少 3 case：渲染 / 保存 / 测试 |
| T5.9 | `agent-progress.md` + `feature_list.json` 状态更新 | 单文件 | 本轮 session log + verification 证据 |
| T5.10 | `just precommit` 全绿 | 全套检查 | 0 error 0 warning |

**Review Checkpoint**：用户配 ENScan AQC cookie + 跑 enscan -n 小米 -type aqc 真实成功；0.zone API key 通过新 UI 重配 + intel_query_provider 返回结果；GitHub 工具安装不再 403。

---

## 任务依赖图

```
Phase 1 (schema 类型) → Phase 2 (backends) ─┐
                                            ├─ Phase 3 (IPC) → Phase 4 (UI) → Phase 5 (接入+迁移+E2E)
                                            │
T1.6 (toolconfig schema) ─────────────────┘
T1.7 (provider meta) ─────────────────────┘
```

---

## 风险与缓解

| 风险 | 概率 | 缓解 |
|---|---|---|
| ENScan_GO 的 config.yaml 真实 schema 与文档示例不一致 | 中 | 先 `enscan -v` 生成一份样本，对照写 mapper；preserve_unknown_keys 兜底 |
| 现有 vault entries 的 read alias 漏掉某种情况 | 中 | 单测覆盖；用户审核 Phase 5 时人工验 0.zone 旧 key |
| ts-rs 类型同步漏掉 | 低 | Phase 3 review 时验证 frontend/lib/generated/ 含新类型 |
| 前端动态表单的 controlled state 在频繁 keystroke 下卡顿 | 低 | debounce save；不要 onChange invoke 后端 |
| external_file 写入时 ENScan 正在读，造成 cookie 短暂不可用 | 低 | atomic rename + tmp 文件保证一致性 |
| `golish-integrations` crate 引入 reqwest 与现有 vendored fork 冲突 | 低 | 使用 workspace.dependencies 复用 |
| 删除老 `IntelProvidersSettings` 导致 i18n key 找不到 | 中 | 保留 `intel.*` 翻译键半个版本周期 fallback |

---

## 不在本计划范围

- OS keychain（macOS Keychain / Windows Credential Manager / Linux libsecret）— 下一轮
- AI provider key（OpenAI / Anthropic / Vertex）迁入 Integrations — 用户明确不做
- Per-project credential override（按 project_path 隔离）— 不做
- Cookie 自动刷新（无头浏览器登录）— 不做
- 移除 vault 的 `intel-provider` 老 tag — 保留兼容期半个版本

---

## 验收清单（每个 Phase 结束 mandatory）

```
[ ] Phase 1: schema 类型定义 + cargo check 通过 + 用户审 type signature
[ ] Phase 2: 3 个 storage backend + 所有单测绿 + 用户人工验 ExternalFileBackend
[ ] Phase 3: 5 个 IPC 命令 + ts-rs 类型同步 + devtools 能 invoke
[ ] Phase 4: 前端 UI 三态完备 + 截图 review
[ ] Phase 5: 端到端验证 + 删老 UI + 用户配真实 cookie 跑通 ENScan
[ ] 最终: just precommit 全绿 + feature_list.json 切 passing
```

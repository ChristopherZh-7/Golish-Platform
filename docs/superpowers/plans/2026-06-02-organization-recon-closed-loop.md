# Organization Recon 闭环实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 在不改数据库 schema、不发起未经确认的真实外部请求的前提下，把企业信息、被动资产、主动工具、规范化处理和幂等入库串成可重放的 Organization Recon 闭环。

**架构：** 复用现有 `asset_intel` 作为 ENScan_GO 和 `0.zone` adapter，在 `golish-recon-app` 新增 `organization_recon` 编排层。编排层立即返回 run id，后台执行阶段屏障和有界并发；CLI / HTTP 来源通过 artifact manifest 交付结果，主动 pipeline MVP 保存结果汇总 artifact，运行历史落 filesystem manifest + 现有 `audit_log`。

**技术栈：** Rust 2021、Tokio、Tauri 2、serde/serde_json、现有 `golish-projects` file storage、现有 `golish-pipeline` parser/storage 契约、React/TypeScript。

## 文件结构

### 创建

- `backend/crates/golish-recon-app/src/organization_recon/mod.rs`：模块出口、Tauri 命令重导出。
- `backend/crates/golish-recon-app/src/organization_recon/types.rs`：run、stage、task、manifest、normalized record DTO。
- `backend/crates/golish-recon-app/src/organization_recon/artifacts.rs`：raw/normalized/manifest 原子写入和 UTF-8 清理。
- `backend/crates/golish-recon-app/src/organization_recon/normalize.rs`：字段规范化、幂等 key、多来源 evidence 合并。
- `backend/crates/golish-recon-app/src/organization_recon/runner.rs`：阶段屏障、有界并发、adapter 调度、audit 摘要。
- `backend/crates/golish-recon-app/src/organization_recon/state.rs`：Tauri-managed run 快照。
- `backend/crates/golish-recon-app/src/organization_recon/commands.rs`：`organization_recon_start_run`、`organization_recon_get_run`。
- `backend/crates/golish/src/commands_facade/organization_recon.rs`：命令 facade。
- `frontend/lib/api/organization-recon.ts`：IPC wrapper 和统一事件类型。
- `frontend/lib/target-panel/organization-recon.ts`：按 run id 隔离延迟事件。
- `frontend/lib/target-panel/organization-recon.test.ts`：前端并发 run 回归测试。
- `frontend/lib/generated/{AssetIntelHydrateConfig,OrganizationRecon*ReconTask*}.ts`：ts-rs 生成 IPC 绑定。

### 修改

- `justfile`：让 `just step` 在子 recipe 失败时退出非零。
- `backend/crates/golish-integrations/src/schema/storage.rs`：external file 通用 defaults。
- `backend/crates/golish-integrations/src/storage/external_file.rs`：先写 defaults，再覆盖用户字段。
- `backend/crates/golish-integrations/src/storage/external_file_tests.rs`：defaults 回归测试。
- `resources/toolsconfig/enscan-go.json`：`cookies.aqc` 和 `version: "0.7"`。
- `backend/crates/golish-recon-app/src/asset_intel/runtime/cli/{mod.rs,stream.rs}`：raw stdout/stderr、artifact 解析失败显式化、manifest。
- `backend/crates/golish-recon-app/src/asset_intel/runtime/http.rs`：raw response、request id、HTTP 错误分类、manifest。
- `backend/crates/golish-recon-app/src/asset_intel/service/hydrate.rs`：把项目目录传给 HTTP adapter，修复仅 profile 数据被误判 `checked_empty`，并按 CLI=2 / HTTP=4 做 provider 有界并发。
- `backend/crates/golish-pipeline/src/parser.rs`：regex fields 兼容旧方向。
- `backend/crates/golish-pentest-app/src/output_parser.rs`：前端手动 parser 同步兼容旧方向。
- `resources/toolsconfig/{subfinder,amass,nmap,httpx}.json`：canonical `db_action` 和 regex fields。
- `backend/crates/golish-recon-app/src/lib.rs`：导出 Organization Recon。
- `backend/crates/golish/src/app/tauri_app.rs`：manage `OrganizationReconState`。
- `backend/crates/golish/src/commands_facade/mod.rs`、`backend/crates/golish/src/commands_registry.rs`：注册新命令。
- `frontend/lib/api/index.ts`：导出 wrapper。
- `frontend/components/TargetPanel/{AssetIntelActivityPanel,OrgWorkspacePanel,TargetGroupedView}.tsx`：启动 staged run、展示阶段状态、按 run id 过滤流事件。
- `biome.json`：generated 目录关闭 assist import 排序，避免 Biome 重排 ts-rs 生成文件。

## Task 1：恢复可信基线

**文件：**
- 修改：`justfile`
- 自动格式化：`frontend/lib/generated/ToolConfig.ts`
- 自动格式化：`frontend/lib/pentest/types.ts`

**步骤：**
1. 给 `[private] step recipe` 增加 `set -euo pipefail`。
2. 使用 Biome 自动整理两处 import，不手写生成文件。
3. 运行 `./init.sh --skip-install`，确认任何内部失败都能让脚本退出非零。

**验证：**
```bash
./init.sh --skip-install
```

预期：所有 stage 真正通过；若仍失败，命令退出非零并显示第一处失败。

**提交：** `fix(harness): propagate staged check failures`

## Task 2：修复 ENScan external-file 配置

**文件：**
- 修改：`backend/crates/golish-integrations/src/schema/storage.rs`
- 修改：`backend/crates/golish-integrations/src/storage/external_file.rs`
- 修改：`backend/crates/golish-integrations/src/storage/external_file_tests.rs`
- 修改：`resources/toolsconfig/enscan-go.json`

**步骤：**
1. 在 `ExternalFileStorage` 增加：
```rust
#[serde(default, skip_serializing_if = "HashMap::is_empty")]
pub defaults: HashMap<String, String>,
```
2. 在 `write()` 加载文档后，先逐项 `set_at_path(&mut doc, key, value)` 写 defaults，再写用户 fields，让显式字段覆盖默认值。
3. 把 ENScan AQC 的 field、capture target、说明统一改为 `cookies.aqc`。
4. 在 ENScan `external_file` 增加：
```json
"defaults": { "version": "0.7" }
```
5. 增加 YAML 测试，断言首次写入和更新写入都保留 `version: "0.7"`。

**验证：**
```bash
cd backend && cargo nextest run -p golish-integrations external_file --status-level fail
```

预期：external-file 测试全部通过。

**提交：** `fix(recon): write enscan aqc cookie and config defaults`

## Task 3：落地 artifact 和防乱码契约

**文件：**
- 创建：`backend/crates/golish-recon-app/src/organization_recon/artifacts.rs`
- 创建：`backend/crates/golish-recon-app/src/organization_recon/types.rs`
- 修改：`backend/crates/golish-recon-app/src/asset_intel/runtime/cli/mod.rs`
- 修改：`backend/crates/golish-recon-app/src/asset_intel/runtime/cli/stream.rs`
- 修改：`backend/crates/golish-recon-app/src/asset_intel/runtime/http.rs`
- 修改：`backend/crates/golish-recon-app/src/asset_intel/service/hydrate.rs`
- 修改：`backend/crates/golish-recon-app/src/asset_intel/tests.rs`

**步骤：**
1. 定义 `ReconTaskManifest`、`ReconArtifactRef`、`ReconTaskError`。
2. 实现 `write_raw_bytes()`、`decode_utf8_clean()`、`write_json_manifest()`、`write_records_jsonl()`。
3. CLI runner 独立收集 stdout/stderr，退出后写 `raw/stdout.log`、`raw/stderr.log` 和 manifest。
4. HTTP runner 接收 `project_root`，每个 request 将 body 原样写为 `raw/response-<request_id>.json`。
5. HTTP 错误映射成 `timeout | unauthorized | quota_exceeded | rate_limited | server_error | transport_error | parse_error`。
6. 最终状态判断使用 `candidate_count + profile_entries.len()`，避免只有 profile 数据时误报 `checked_empty`。
7. 增加非法 UTF-8、坏 JSON、profile-only、空结果测试。

**验证：**
```bash
cd backend && cargo nextest run -p golish-recon-app asset_intel --status-level fail
```

预期：asset-intel 测试全部通过，临时目录包含 raw 和 manifest。

**提交：** `feat(recon): persist source artifacts and explicit failure states`

## Task 4：修复主动工具解析契约

**文件：**
- 修改：`backend/crates/golish-pentest/src/output_parser.rs`
- 修改：`backend/crates/golish-pipeline/src/parser.rs`
- 修改：`backend/crates/golish-pentest-app/src/output_parser.rs`
- 修改：`resources/toolsconfig/subfinder.json`
- 修改：`resources/toolsconfig/amass.json`
- 修改：`resources/toolsconfig/nmap.json`
- 修改：`resources/toolsconfig/httpx.json`

**步骤：**
1. 在 `golish-pentest::output_parser` 增加共享 helper：优先识别 canonical `field -> "$1"`，再兼容 legacy `"1" -> "field"`。
2. 两个消费 parser 的 text 路径统一使用该 helper。
3. 配置迁移为：
```text
subfinder db_action = target_add
amass     db_action = target_add
nmap      db_action = target_update_recon
httpx     db_action = target_update_recon
```
4. 把 text pattern fields 改为 `"hostname": "$1"`、`"port": "$1"` 等 canonical 形式。
5. 增加 canonical 和 legacy 两组 parser 测试。

**验证：**
```bash
cd backend && cargo nextest run -p golish-pipeline parser --status-level fail
cd backend && cargo nextest run -p golish-pentest-app output_parser --status-level fail
```

预期：canonical 和 legacy 都可提取字段。

**提交：** `fix(recon): align active tool output parser contracts`

## Task 5：实现 normalized record 处理层

**文件：**
- 创建：`backend/crates/golish-recon-app/src/organization_recon/normalize.rs`
- 修改：`backend/crates/golish-recon-app/src/organization_recon/types.rs`
- 创建：`backend/crates/golish-recon-app/src/organization_recon/mod.rs`

**步骤：**
1. 定义 `ReconRecordKind`、`NormalizedReconRecord`、`ReconEvidenceRef`。
2. 实现 domain、IP、port、URL、organization key 规则。
3. 实现 `merge_normalized_records()`，相同 key 合并 evidence。
4. 测试大小写域名、尾点域名、IPv6、默认端口 URL、多来源 evidence。

**验证：**
```bash
cd backend && cargo nextest run -p golish-recon-app organization_recon::normalize --status-level fail
```

预期：normalized record 单测全部通过。

**提交：** `feat(recon): normalize records with evidence-preserving dedupe`

## Task 6：实现后台 Recon Run 编排

**文件：**
- 修改：`backend/crates/golish-recon-app/Cargo.toml`
- 创建：`backend/crates/golish-recon-app/src/organization_recon/state.rs`
- 创建：`backend/crates/golish-recon-app/src/organization_recon/runner.rs`
- 创建：`backend/crates/golish-recon-app/src/organization_recon/storage.rs`
- 创建：`backend/crates/golish-recon-app/src/organization_recon/commands.rs`
- 修改：`backend/crates/golish-recon-app/src/lib.rs`
- 修改：`backend/crates/golish/src/app/tauri_app.rs`
- 创建：`backend/crates/golish/src/commands_facade/organization_recon.rs`
- 修改：`backend/crates/golish/src/commands_facade/mod.rs`
- 修改：`backend/crates/golish/src/commands_registry.rs`

**步骤：**
1. 定义 `OrganizationReconState` 和 `OrganizationReconRunSnapshot`。
2. 定义 `organization_recon_start_run`：校验组织存在，若启用主动阶段则校验当前项目 in-scope targets，插入 queued 快照，`tokio::spawn` 后台执行并立即返回 run id。
3. 定义 `organization_recon_get_run`：按 run id 返回快照。
4. runner 按 `enterprise_intel -> passive_internet -> active_collection -> processing -> persistence` 执行；asset provider 使用 CLI `Semaphore(2)` / HTTP `Semaphore(4)`，主动目标使用 `buffer_unordered(2)`，DNS 工具由 pipeline DAG 同层并发。
5. 每次 task 状态变化写 manifest，发 `organization-recon:event`，最后用 `audit_log` 写摘要。
6. 使用纯本地 stage-state fixture 测试阶段顺序、部分失败回退、`checked_empty` 和根 manifest；使用 DAG 测试锁住 DNS 同层、domain-only 输入和 `httpx` 端口迭代。

**验证：**
```bash
cd backend && cargo nextest run -p golish-recon-app organization_recon --status-level fail
cd backend && cargo check -p golish
```

预期：编排测试通过，聚合命令注册可编译。

**提交：** `feat(recon): add asynchronous organization recon orchestration`

## Task 7：接前端统一进度

**文件：**
- 创建：`frontend/lib/api/organization-recon.ts`
- 创建：`frontend/lib/target-panel/organization-recon.ts`
- 创建：`frontend/lib/target-panel/organization-recon.test.ts`
- 生成：`frontend/lib/generated/{AssetIntelHydrateConfig,OrganizationRecon*ReconTask*}.ts`
- 修改：`frontend/lib/api/index.ts`
- 修改：`frontend/components/TargetPanel/AssetIntelActivityPanel.tsx`
- 修改：`frontend/components/TargetPanel/OrgWorkspacePanel.tsx`
- 修改：`frontend/components/TargetPanel/TargetGroupedView.tsx`

**步骤：**
1. 增加 `organizationRecon.startRun()`、`organizationRecon.getRun()`、`organizationRecon.listenStream()`。
2. 在现有 asset-intel reducer 入口先判断：已有 `runId` 且事件 `runId` 不同则忽略。
3. 增加并发 run 测试，证明 org A 的事件不会写进 org B activity。
4. 在 Activity tab 展示阶段状态和失败码；主动阶段默认关闭。

**验证：**
```bash
just check-fe
just test-fe
```

预期：Biome、typecheck、Vitest 全部通过。

**提交：** `feat(recon-ui): show staged organization recon progress`

## Task 8：闭环验收和收尾

**文件：**
- 修改：`agent-progress.md`
- 修改：`feature_list.json`

**步骤：**
1. 用纯本地 stage-state fixture 执行五阶段 manifest / 回退测试。
2. 运行 focused tests、`just arch`、`just precommit`。
3. 在 `agent-progress.md` 记录命令、退出码和关键输出。
4. 本地门禁全绿后仍保持 `in_progress`，直到 active pipeline 逐 step stdout / stderr bytes 纳入正式 artifact。
5. 真实 AQC、`0.zone`、授权 active scan 联调保持为单独确认后的验收批次；全部证据齐全后再切 `passing`。

**验证：**
```bash
just arch
just precommit
git diff --check
```

预期：全部退出 0。

**提交：** `docs(recon): record closed-loop verification evidence`

# Organization Recon 闭环设计

## 1. 目标

本轮把 `.learndocs/enrich-organization-next-work-record-2026-06-01.md` 第五节定义的流程做成可运行闭环：

1. 企业信息收集：ENScan_GO。
2. 企业互联网资产被动收集：暂时只接 `0.zone`。
3. 主动收集：`subfinder + amass`、`nmap`、`httpx`。
4. 信息处理：统一字段、UTF-8 防乱码、规范化、去重、多来源 evidence。
5. 信息入库：复用现有业务表，失败不入业务数据，空结果保留 `checked_empty` evidence。

完成度打分不在本轮范围内。

## 2. 安全边界

### 2.1 不做隐式外部请求

自动测试只使用本地 fixture 和临时目录，不请求真实企业情报 API，不读取真实 Cookie，不扫描公网目标。

真实联调分三类，均需用户单独确认后执行：

- ENScan_GO AQC 正向、Cookie 过期、无 Cookie、旧配置升级样本。
- `0.zone` quota、401/403、429、5xx、timeout 样本。
- 对明确授权 scope 的 `subfinder`、`amass`、`nmap`、`httpx` 扫描。

### 2.2 主动扫描必须显式授权

主动阶段默认关闭。启动参数必须携带 `allow_active=true`，且每个待扫描目标必须已存在于当前项目 `targets` 表并处于 `scope=in`。后端独立校验，不依赖前端按钮状态。

### 2.3 本轮不改 schema

MVP 不增加 `recon_runs`、`recon_source_tasks` 表。运行状态写入：

```text
<project>/.golish/tool-output/recon/<run_id>/manifest.json
<project>/.golish/tool-output/recon/<run_id>/<stage>/<source_id>/manifest.json
```

摘要复用现有 `audit_log`。需要跨重启恢复、历史分页和断点续跑时，再单独设计 migration 并向用户确认。

## 3. 运行模型

```text
OrganizationReconRun
  run_id
  organization_id
  project_path
  status
  stages[]

ReconSourceTask
  task_id
  run_id
  stage
  source_id
  source_kind       online_api | cli_tool | processor | persistence
  status            queued | running | completed | checked_empty |
                    failed | skipped | cancelled
  artifact_dir
  progress
  failure_code
  failure_message
```

后台状态保存在 Tauri-managed `OrganizationReconState`。`organization_recon_start_run` 完成参数校验后立即返回 `run_id`，实际任务由 `tokio::spawn` 执行。前端通过统一事件查看阶段和 source task 进度。

## 4. Artifact 契约

CLI / HTTP 来源目录固定为：

```text
raw/
  stdout.log
  stderr.log
  response-*.json
normalized/
  records.jsonl
manifest.json
```

来源 manifest 至少包含：

```json
{
  "runId": "uuid",
  "taskId": "uuid",
  "stage": "enterprise_intel",
  "sourceId": "enscan-go",
  "status": "completed",
  "exitCode": 0,
  "encoding": "utf-8",
  "artifacts": [],
  "recordCount": 1,
  "checkedEmpty": false,
  "errors": []
}
```

CLI / HTTP 原始 bytes 必须先落 `raw/`，再做解码和解析。数据库只接收清理后的 UTF-8 记录。解码失败、JSON 失败、空结果分别记录，不能静默跳过。

主动工具 MVP 复用现有 `golish-pipeline`：当前把每个目标的 pipeline 结果汇总写入 `active_collection/active-collection/raw/pipeline-results.json`，并在业务表和 audit log 保留解析后的端口、站点信息。逐 step 的原始 stdout / stderr bytes 仍需在后续 hardening 中从 pipeline 临时目录提升为正式 artifact；在此之前不能把主动阶段描述成完整 raw byte ledger。

## 5. 适配器

### 5.1 企业信息

现有 `asset_intel` CLI runtime 继续承载 ENScan_GO。修复 AQC 字段为 `cookies.aqc`，external-file schema 增加通用 `defaults`，配置声明 `version: "0.7"`。

### 5.2 被动互联网资产

现有 `asset_intel` HTTP runtime 继续承载 `0.zone`。每个请求保存原始 response，保留 request id，并把错误分类为：

```text
timeout | unauthorized | quota_exceeded | rate_limited |
server_error | transport_error | parse_error
```

### 5.3 主动工具

主动工具不塞进 `AssetIntelRuntimeConfig`。使用 Recon CLI source adapter：

- DNS：`subfinder`、`amass` 可并发。
- 端口：`nmap` 在 DNS 结果后运行。
- 站点探活：`httpx` 在端口结果后运行。

`toolsconfig` 是工具元数据源。历史 `host_add`、`endpoint_add` 改成 parser 已支持的 canonical `db_action`；regex fields 统一写成 `field_name -> "$capture_group"`，同时 parser 兼容旧配置。

## 6. 标准字段

normalized record 使用以下字段类型：

```text
organization | domain | ip | port | service | url | site |
certificate | contact | leak
```

每条记录包含：

```text
record_id
kind
key
value
attributes
evidence[]
```

幂等键规则：

- `organization`：credit code 优先，否则规范化名称。
- `domain`：lowercase，去尾点。
- `ip`：标准库 `IpAddr` 解析后的字符串。
- `port`：`ip + protocol + port`。
- `url`：标准化 scheme、host、port、path。
- 其余类型：类型前缀 + 规范化 value。

去重时合并 evidence，不覆盖来源。

## 7. 并发与回退

编排层按阶段屏障执行，阶段内使用有界并发。当前实现中 CLI provider 由 `Semaphore(2)` 限流、HTTP provider 由 `Semaphore(4)` 限流、主动目标由 `buffer_unordered(2)` 限流；pipeline DAG 同层的 `subfinder` / `amass` 并发执行。

| 类型 | 默认并发 |
|---|---:|
| 企业来源 | 2 |
| 网络空间测绘 API | 4 |
| DNS 来源 | 4 |
| 端口扫描 | 2 |
| HTTP 探活 | 8 |

单个来源失败不抹掉其它来源结果。阶段状态按 source task 汇总：

- 全部成功或 `checked_empty`：`completed`。
- 部分失败：`partial`。
- 全部失败：`failed`。

## 8. 验收

本地自动验证证明：

1. 五个阶段按顺序写 task manifest；被动来源失败后，后续主动空结果、处理和持久化阶段仍继续，根 manifest 为 `partial`。
2. DNS 同阶段并发启动。
3. 主动阶段默认关闭，启用时后端要求当前组织至少存在一个 `scope=in` target。
4. 非 UTF-8、坏 JSON、空结果状态互不混淆。
5. 同一 normalized record 多来源合并 evidence。
6. manifest 可重放，Tauri IPC 类型由 ts-rs 单源生成。

真实联调仍需用户单独授权后验收：

1. AQC 正向、过期 Cookie、无 Cookie 和旧配置升级。
2. `0.zone` quota、401/403、429、5xx 和 timeout。
3. 明确授权 scope 的主动扫描，并把逐 step stdout / stderr bytes 纳入正式 artifact。

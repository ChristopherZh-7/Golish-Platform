# Target Surface Fingerprint / Network-Failure Closure 实现计划

> 使用 test-driven-development：每个 seam 先建立失败断言，再做最小实现；本轮不自动 commit/push。

**目标：** 修复 Target Surface 数据可见性，并让 exact Web Origin 在连续三次稳定网络/TLS失败、且独立复核仍不可达后，形成可审计的 EAS 终态及 Enumeration 排除闭环。

## Task 1：目录查询与部分加载

- 在 `golish-db/repo/directory_entries.rs` 先加强 JOIN SQL 测试，要求所有投影列均为 `de.*`，再修查询。
- 在 `useTargetSurfaceData` 测试中复现 directory source rejection 仍保留 fingerprints/origins；实现逐源 settle 与结构化 source errors。
- 保持 loading/error/empty 三态，不让单源错误清空整页。

## Task 2：指纹写入与 exact-origin 归属

- 在 `golish-pentest/output_store/targets.rs` 先测试 WhatWeb evidence 包含 canonical exact origin，且相同技术的多个 origin observation 不丢失。
- 写入数组 evidence，并兼容已有 upsert 语义。
- 在 frontend normalization / hierarchy 测试中先复现 object evidence 被丢弃、只挂第一个 origin、confidence 显示错误，再实现兼容。
- IP Target 增加可见的 target-level 指纹入口；明确分开展示已归属与 unassigned 历史指纹。

## Task 3：连续三次失败计数

- 在 `operation_state` repo 添加原子 JSONB namespace 更新 API 和并发/隔离 SQL 合同测试，不做 schema 变更。
- 在 EAS WhatWeb wrapper 先测试同 key 同类失败依次得到 attempt 1/2 error、attempt 3 producer blocked；不同 origin/target/class 不串计数。
- 测试 HTTP 响应/成功会清计数，未知 stderr/工具/DB 错误不计数。
- 将当前“一次 EOF/reset 即 blocked”收紧为三次合同。

## Task 4：独立复核与 Enumeration handoff

- 抽取/复用固定 HEAD + bounded GET、direct/proxy transport policy，让 EAS 第三次失败时执行独立复核。
- 先测试 WhatWeb-only blocked 不得排除 Enumeration；只有 guarded
  `web_origin_transport_blocked` exact-origin handoff 才能排除。
- 在 EAS stage coverage / worklist 与 Enumeration parent/exact-origin expansion、org gate 中保持同一可信事实。
- 测试同 IP:port 的 sibling Host/SNI origin 仍被纳入，且 target/open-port durable truth不被修改。

## Task 5：合同、模块卡与聚焦验证

- 在 Target Evidence tab 先测试 WhatWeb transport audit 的 attempt/class/outcome/origin
  不可见，再用现有 `detail.raw_output` 渲染 attempt 1/2 网络错误、attempt 3 producer
  stop，以及仅在 independently-confirmed 时显示 Enumeration exact-origin exclusion。
- 更新 EAS / Enumeration methodology、相关 backend/frontend 模块卡与 INDEX。
- 更新 `feature_list.json`、`agent-progress.md`，记录 RED→GREEN 命令、退出码和关键结果。
- 运行受影响 crate/frontend 聚焦测试、scoped Clippy、rustfmt、typecheck/Biome、JSON parse 与 `git diff --check`。
- 遵守当前约束，不运行 `./init.sh` / full `just precommit`，不发起真实外部扫描；fresh compiled live run 之前状态保持 `in_progress`。

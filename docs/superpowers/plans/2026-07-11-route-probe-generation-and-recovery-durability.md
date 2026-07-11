# Route Probe Generation 与恢复持久性实现计划

## 目标

让 DIR producer 在 timeout、DB 瞬时失败、稳定持久化失败、attempt supersede 与进程
崩溃下都保持事实一致并有限收敛。

## 实现步骤

1. 将 route checkpoint 升为 v8，加入 pending business write、terminal cursor、三类
   failure fingerprint/counter，并对大小、重复 URL、origin、overlap 和 witness 做
   fail-closed validation。
2. checkpoint store/clear 改为短事务：锁 operation epoch 后重新读取 current generation，
   再更新 JSONB slot；SQL/read outage 不清合法 cursor。
3. 扩展 `golish-db::directory_entries` 与 `ReconDirectoryPort`，让 route business write
   在 target + epoch + org subtree + generation CAS 下返回 `Applied|Superseded`。
4. 扩展 `technique_outcomes` conditional API，把 terminal outcome 与 checkpoint slot
   clear 放进同一事务，并限制该 API 只接收 `found|empty|blocked`。
5. 对 candidate、pending write、terminal publication 分别实现 bounded breaker；只有
   candidate recovery exhaustion 可产生 evidence-backed DIR blocked，两个 persistence
   breaker 只停止自动 retry，并提供显式 post-repair flag。
6. 按 absolute URL 去重 candidate，按 exact origin 去重 batch；保留 wordlist recursion
   witness。单 root completion runtime 维持独立 30 分钟上限；batch completion 默认
   不设 scheduling-start ceiling，显式 ceiling 只跳过尚未启动的 roots。扩大 cursor
   capacity 并给 overflow 可执行诊断。
7. 将 terminal publication 从模糊 bool 改成 `Published|Superseded|Failed(kind, detail)`；
   breaker 按真实 kind 统计，所有非终态且 `automatic_retry_allowed=false` 的结果都输出
   `manual_repair_reason + recovery_action`。
8. 同步 bridge bounded summary、sub-agent model compactor、Enumerator prompt、stage
   methodology、模块卡和设计文档。

## 验证

```bash
cd backend
CARGO_INCREMENTAL=0 cargo nextest run -p golish-pentest-app route_probe_paths
CARGO_INCREMENTAL=0 cargo nextest run -p golish-sub-agents route_probe_
CARGO_INCREMENTAL=0 cargo nextest run -p golish-db technique_outcomes
cargo clippy -p golish-db -p golish-app-core -p golish-pentest-app -p golish-sub-agents --all-targets -- -D warnings
```

随后重建 `golish`，在 Test1 运行完整 Enumeration，并用 `scripts/run_tree.py --full --db`
确认所有 exact-origin 四轴终态、无 `_truncated_entry`、无旧 generation 写入和 gate PASS。

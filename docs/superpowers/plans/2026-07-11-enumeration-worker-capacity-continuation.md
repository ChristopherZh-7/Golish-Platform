# Enumeration worker capacity continuation implementation plan

1. 为 372 roots / 8 worklist pages 写回归测试，锁定 7 次分页续段与 hard cap 8。
2. 为 coverage-only deliverable BLOCK 和无 deliverable 写 RED 测试：strict progress、
   stall、mixed blocker、同数量不同 cell key、chain missing、budget exhausted 与独立
   ready-submit-only。
3. 在 `stage_run_call.rs` 从 raw `stage_asset_coverage` snapshot 确定性统计 root count 与
   `pending + error + partial`，并在每个 segment 前后比较。
4. 只有 gate gap key set 与 authoritative unfinished key set 完全相等且严格前进时复用
   exact worker chain，不增加 gate attempt；无进展、取消、缺链或耗尽时进入现有
   request-scoped breaker，混合或 stale blocker 保留普通 gate repair。
5. 更新 runtime 模块卡并运行 focused nextest、整 crate nextest、fmt、clippy。

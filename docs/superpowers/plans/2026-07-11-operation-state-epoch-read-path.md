# Operation state epoch-only read path implementation plan

1. 先为 epoch SQL 投影与 row 形状增加失败测试，确认现有 repo 缺少窄读取契约。
2. 在 `golish-db::repo::operation_state` 增加 `OperationEpochRow` 与 `get_epoch`，查询只取
   stage-attempt 有效性需要的五列。
3. 将 Enumeration preflight、browser collector、JS extractor 与 route checkpoint identity
   的 active-operation 读取切换到 `get_epoch`，不改变现有校验分支。
4. 更新 `golish-db/repo` 与 `golish-pentest-app/pentest_bridge` 模块卡，记录完整读取与
   epoch-only 读取的边界。
5. 运行 focused nextest、相关 crate clippy、workspace fmt check 和 `git diff --check`。

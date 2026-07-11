# Stage purge 原子性与 technique 精确作用域实现计划

1. 先补 RED tests：要求 technique outcome SQL 同时按 org/technique 过滤，并要求 affected stage technique 集合来自 embedded specs 的并集。
2. 将 `stage_purge` executors 从 `PgPool` 改为 `&mut PgConnection`，新增精确 technique outcome delete。
3. 在 `harness_dev::purge_stage_facts` 建立单一 transaction；所有 domain、ledger、wave、status 步骤成功才 commit，错误显式 rollback。
4. 更新设计说明与 `golish-db/repo`、`golish-agent-app/ai` 模块卡。
5. 跑 focused tests、fmt、clippy 与 diff check；不执行真实 purge、不改 schema、不 commit。

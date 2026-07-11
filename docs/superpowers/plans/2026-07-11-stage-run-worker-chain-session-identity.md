# Stage-run worker chain session identity implementation plan

1. 为 `chain_persist` 写 RED 测试，复现 `stage-run-*` event key 令 chain create 静默失效。
2. 在 `SubAgentExecutorContext` 增加独立 `persistence_session_id`，chain restore/create
   优先使用它，并保留 legacy UUID event-key 回退。
3. 从 runtime/bridge 的 DB tracker 注入真实 session UUID；reflector、nested delegate、
   eval/test 构造点显式透传或置空。
4. 将 exact/latest resume 改为 fail closed：ownership mismatch、miss、DB error、非法
   resume 或损坏 history 均不得创建 fresh chain；Postgres load/update 校验 owner/行数。
5. 只有 durable history write 成功才输出 chain marker，并把 typed chain failure 传到
   `stage_run`；Finalize 等已可能有副作用的失败直接停止，不进入普通 gate retry。
6. 验证最终 sub-agent response 带精确 chain marker，capacity continuation 不再因
   `has_resume_chain=false` 被误判 exhausted。
7. 更新 runtime/sub-agents/bridge 模块卡，运行 focused/full nextest、clippy、fmt、
   diff check，再做 fresh Test1 Enumeration live closeout。

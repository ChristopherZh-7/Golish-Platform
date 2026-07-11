# Session generation ownership and history recovery implementation plan

> 设计：`docs/design/2026-07-10-session-generation-and-history-recovery.md`

## Goal

把 universal request owner 提升为跨 bridge generation 的 logical-session 合同，并让 Task
isolated history 在 success/error/Stop/abort/panic 下都可恢复；补齐 compaction/full restore
与 frontend clear ingress。

## Steps

1. 在 `golish-agent-bridge` 实现 stable `SessionRequestSlot`、generation-bound request
   lease、lifecycle transition lease 与 foreign-generation 校验。
2. 在 `AiState` 保存 stable slot + per-session lifecycle；init 构建前 fail-fast reserve，
   publish 时 activate/bind/replace；shutdown invalidate/remove 后 cancel。
3. 为 isolated history 增加同步 recovery slot；normal path restore，next begin 恢复
   abort/panic backup。
4. 给 `retry_compaction` 和 full `restore_ai_session` 接 universal owner/cleanup。
5. 调整 frontend `clearConversation` 为 backend-first，并收窄 legacy fallback。
6. 补 generation、replacement、shutdown、foreign lease、history abort/panic、busy mutation、
   frontend atomicity 测试。
7. 同步 module cards，运行 focused nextest/Vitest、check、Clippy、rustfmt 与 diff check。

## Completion evidence

- `CARGO_INCREMENTAL=0 cargo nextest run -p golish-agent-bridge`
- `CARGO_INCREMENTAL=0 cargo nextest run -p golish-agent-app session_slot_tests`
- `pnpm exec vitest run frontend/store/actions.clear-conversation.test.ts`
- `CARGO_INCREMENTAL=0 cargo check -p golish-agent-bridge -p golish-agent-app -p golish`
- `CARGO_INCREMENTAL=0 cargo clippy -p golish-agent-bridge -p golish-agent-app --all-targets -- -D warnings`
- `cargo fmt --all -- --check`
- `git diff --check`

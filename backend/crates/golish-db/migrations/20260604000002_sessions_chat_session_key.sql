-- Task 断线恢复 · L1 锚点：把 chat-panel 的字符串 session id 落到 sessions 行上，
-- 让"同一 chat 会话"稳定对应"同一 DB session"（而不是每条 task 消息新建一行）。
-- 这样入口才能按 chat 会话查到上一个未完成的 operation 并恢复，而不是从 scoping 重来。
--
-- 向后兼容（不变量 I10 扩展式）：
--   * 列可空、IF NOT EXISTS 幂等；已有行 chat_session_key 全 NULL。
--   * PG 唯一索引默认把 NULL 视为互不相同（NULLs distinct），故已有 NULL 行不互撞，
--     新写入的非空 key 才受唯一约束 → upsert(ON CONFLICT) 可用。

ALTER TABLE sessions ADD COLUMN IF NOT EXISTS chat_session_key TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_sessions_chat_session_key
    ON sessions(chat_session_key);

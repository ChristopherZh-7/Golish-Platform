//! Engagement commands facade（chat-native 批量红队，设计 2026-06-13）。
//!
//! - 快照查询（Phase A）：org 树 + DB 真值覆盖 + 薄弱度——scoping 锁定范围后的
//!   「范围已锁定」信号；Phase B 工人池 / Phase C 总览复用同一读模型。
//! - 工人范围（Phase B）：fan-out 池把 spawn 出的工人会话钉到一个 org + 阶段
//!   切片（实现住 golish-agent-app，按域归入 engagement 门面）。

pub use crate::ai::commands::engagement_scope::{
    engagement_clear_worker_scope, engagement_get_worker_scope, engagement_set_worker_scope,
};
pub use crate::engagement::query::engagement_get_snapshot;

#[doc(hidden)]
pub use crate::ai::commands::engagement_scope::{
    __cmd__engagement_clear_worker_scope, __cmd__engagement_get_worker_scope,
    __cmd__engagement_set_worker_scope,
};

//! Engagement 编排域（chat-native 批量红队，设计
//! `docs/design/2026-06-13-engagement-scoping-fanout-redesign.md`）。
//!
//! 取代已 revert 的 headless fleet（`--fleet-run` CLI + 进程内两波调度，存
//! `git stash`）；本模块只保留与执行形态解耦的**纯逻辑 / 契约**层，搬回时按新
//! 设计重命名 fleet → engagement：
//!
//! - [`scheduler`]：stage-agnostic 调度内核（K 受控并发 / 续跑跳过 / 失败隔离，
//!   trait 注入零 IO）。Phase A 仅搬回备用；Phase B 的前端会话工人池经
//!   Tauri command 注入 executor/oracle/scorer 后驱动它。
//! - [`weakness`]：org 薄弱度评分 + `org_stage_has_truth` 续跑判定 oracle
//!   （计数 SQL 住 `golish_db::repo::engagement_truth`，本层只做权重和阶段映射）。
//! - [`contract`]：前端数据契约（ts-rs 导出 4 个 DTO，I5）。
//! - [`query`]：`engagement_get_snapshot` 只读查询命令——scoping 锁定范围后的
//!   「范围已锁定」信号 + 总览读模型（Phase C 的 scoping 对话升级总览用它渲染）。

pub mod contract;
pub mod query;
pub mod scheduler;
pub mod weakness;

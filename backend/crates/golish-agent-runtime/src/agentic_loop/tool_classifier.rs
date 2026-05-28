//! Tool call classifier · Phase 1b · Doc 2 §5 evidence_read flooding defence.
//!
//! Doc 2 §5 motivation:
//!
//! > 防 agent 通过疯狂 `read_evidence(eid)` 把所有 evidence 拉进上下文绕过
//! > sanitize 隔离.
//!
//! Phase 1 MVP design notes:
//!
//! - 本 module 是**纯 deterministic 规则层**, 不调任何 IO, 单测全部 in-memory.
//! - `RecentToolCallTracker` 用滑动窗口 (`VecDeque<Instant>`) 在 per-session
//!   `RwLock` 后面跑, 避免给 hot path 加 DB 查询.
//! - Phase 1 仅暴露分类函数 + tracker; **不接入** 现有 `tool_dispatch.rs`,
//!   推 Phase 2 (Task 1c.6 之后) 在 `task_orchestrator` 走 stage harness gate
//!   时统一接入. 现在接入风险: 现有 tool dispatch 没有 stage-aware context,
//!   贸然加 classifier 钩子会污染普通 tool 路径.
//! - 超 50 次/min 阈值与 Doc 2 §5 一致.

use std::collections::{HashMap, VecDeque};
use std::sync::RwLock;
use std::time::{Duration, Instant};

/// Doc 2 §5 阈值: 1 分钟内 > 50 次 `evidence_read` 触发警告.
pub const EVIDENCE_READ_THRESHOLD_PER_MINUTE: u32 = 50;

/// Doc 2 §5 评估窗口 (滑动 60s 窗).
pub const EVIDENCE_READ_WINDOW: Duration = Duration::from_secs(60);

/// 评估输入 · 不依赖 rig::completion::ToolCall, 让 classifier 可以独立单测.
#[derive(Debug, Clone)]
pub struct ToolCallProposal<'a> {
    pub name: &'a str,
    pub session_id: &'a str,
}

/// Classifier 警告类型.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCallWarning {
    /// `evidence_read` 频率超阈值. 调用方可决定:
    ///   - 拒绝调用 (soft block, 返 LLM "rate limit exceeded")
    ///   - 警告但放行 (Phase 1 默认)
    ///   - 升级为 stage_harness gate hard block (Phase 2)
    EvidenceReadFlooding {
        count_per_minute: u32,
        threshold: u32,
    },
}

/// 滑动窗口 tracker · per-session.
///
/// 内部用 `VecDeque<Instant>` 存最近 evidence_read 时间戳. 每次 record / classify
/// 时清理 > `EVIDENCE_READ_WINDOW` 前的旧条目.
///
/// 设计决议:
///   - 不用 `tokio::sync::Mutex` (避免 cross-await contention); 用 std `RwLock`,
///     hot path 是 short critical section.
///   - 跨 session 隔离: `HashMap<session_id, VecDeque<Instant>>`. session 在
///     工作流结束后由调用方调 `forget_session` 释放, 否则会缓慢漏内存.
///   - 不持久化: 启动后 in-memory; 死机重启自然清零. 这与 Doc 2 §5 设计意图一致
///     (rate limit 是 ephemeral defence, 不是合规审计).
#[derive(Default)]
pub struct RecentToolCallTracker {
    inner: RwLock<HashMap<String, VecDeque<Instant>>>,
}

impl RecentToolCallTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// 记录一次 evidence_read 调用 (调用方在 dispatch 前调).
    pub fn record_evidence_read(&self, session_id: &str) {
        self.record_evidence_read_at(session_id, Instant::now());
    }

    /// 单测注入用 · 显式 timestamp.
    pub fn record_evidence_read_at(&self, session_id: &str, at: Instant) {
        let mut map = match self.inner.write() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let queue = map.entry(session_id.to_string()).or_default();
        queue.push_back(at);
        prune_outside_window(queue, at);
    }

    /// 当前 session 在过去窗口内的 evidence_read 调用数.
    pub fn count_recent_evidence_reads(&self, session_id: &str) -> u32 {
        self.count_recent_evidence_reads_at(session_id, Instant::now())
    }

    pub fn count_recent_evidence_reads_at(&self, session_id: &str, now: Instant) -> u32 {
        let map = match self.inner.read() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let count = map
            .get(session_id)
            .map(|q| {
                q.iter()
                    .filter(|t| now.saturating_duration_since(**t) <= EVIDENCE_READ_WINDOW)
                    .count()
            })
            .unwrap_or(0);
        u32::try_from(count).unwrap_or(u32::MAX)
    }

    /// session 结束时显式释放 (避免 HashMap 长期堆积).
    pub fn forget_session(&self, session_id: &str) {
        let mut map = match self.inner.write() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        map.remove(session_id);
    }

    /// 全部 sessions 数量 · diagnostic 用.
    pub fn tracked_session_count(&self) -> usize {
        let map = match self.inner.read() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        map.len()
    }
}

fn prune_outside_window(queue: &mut VecDeque<Instant>, now: Instant) {
    while let Some(front) = queue.front() {
        if now.saturating_duration_since(*front) > EVIDENCE_READ_WINDOW {
            queue.pop_front();
        } else {
            break;
        }
    }
}

/// 主 classify 入口 · 给一个 tool call proposal 决定是否警告.
///
/// Phase 1 MVP 仅检查 `evidence_read`. Phase 2 可扩 (e.g. 全局 tool call
/// budget, per-stage forbidden_tools 等).
pub fn classify_tool_call(
    call: &ToolCallProposal<'_>,
    tracker: &RecentToolCallTracker,
) -> Option<ToolCallWarning> {
    if call.name != "evidence_read" {
        return None;
    }

    let count = tracker.count_recent_evidence_reads(call.session_id);
    if count > EVIDENCE_READ_THRESHOLD_PER_MINUTE {
        tracing::warn!(
            target: "harness::tool_classifier",
            session_id = %call.session_id,
            count_per_minute = count,
            threshold = EVIDENCE_READ_THRESHOLD_PER_MINUTE,
            "evidence_read flooding detected"
        );
        Some(ToolCallWarning::EvidenceReadFlooding {
            count_per_minute: count,
            threshold: EVIDENCE_READ_THRESHOLD_PER_MINUTE,
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proposal<'a>(name: &'a str, session_id: &'a str) -> ToolCallProposal<'a> {
        ToolCallProposal { name, session_id }
    }

    #[test]
    fn classify_non_evidence_tool_returns_none() {
        let tracker = RecentToolCallTracker::new();
        let call = proposal("http_probe", "session-1");
        assert_eq!(classify_tool_call(&call, &tracker), None);

        // 即使有 100 次 evidence_read 历史也不影响其他 tool
        for _ in 0..100 {
            tracker.record_evidence_read("session-1");
        }
        assert_eq!(classify_tool_call(&call, &tracker), None);
    }

    #[test]
    fn classify_evidence_read_under_threshold_returns_none() {
        let tracker = RecentToolCallTracker::new();
        for _ in 0..50 {
            tracker.record_evidence_read("session-1");
        }
        let call = proposal("evidence_read", "session-1");
        // count=50, threshold=50 → 不超 (> 50 才警告)
        assert_eq!(classify_tool_call(&call, &tracker), None);
    }

    #[test]
    fn classify_evidence_read_at_threshold_plus_one_returns_warning() {
        let tracker = RecentToolCallTracker::new();
        for _ in 0..51 {
            tracker.record_evidence_read("session-1");
        }
        let call = proposal("evidence_read", "session-1");
        let warning = classify_tool_call(&call, &tracker);
        assert!(warning.is_some());
        match warning.unwrap() {
            ToolCallWarning::EvidenceReadFlooding {
                count_per_minute,
                threshold,
            } => {
                assert_eq!(count_per_minute, 51);
                assert_eq!(threshold, 50);
            }
        }
    }

    #[test]
    fn classify_evidence_read_way_over_returns_warning_with_count() {
        let tracker = RecentToolCallTracker::new();
        for _ in 0..200 {
            tracker.record_evidence_read("session-1");
        }
        let call = proposal("evidence_read", "session-1");
        let warning = classify_tool_call(&call, &tracker);
        assert!(matches!(
            warning,
            Some(ToolCallWarning::EvidenceReadFlooding {
                count_per_minute: 200,
                threshold: 50,
            })
        ));
    }

    #[test]
    fn tracker_isolates_sessions() {
        let tracker = RecentToolCallTracker::new();
        for _ in 0..100 {
            tracker.record_evidence_read("session-A");
        }
        // session-B 完全独立, 没有 evidence_read 记录
        assert_eq!(tracker.count_recent_evidence_reads("session-B"), 0);
        let call_a = proposal("evidence_read", "session-A");
        let call_b = proposal("evidence_read", "session-B");
        assert!(classify_tool_call(&call_a, &tracker).is_some());
        assert!(classify_tool_call(&call_b, &tracker).is_none());
    }

    #[test]
    fn tracker_drops_entries_outside_window() {
        let tracker = RecentToolCallTracker::new();
        let now = Instant::now();
        // 70 个 old entry, 在 70s 前 (window=60s, 全部过期)
        let old = now - Duration::from_secs(70);
        for _ in 0..100 {
            tracker.record_evidence_read_at("session-1", old);
        }
        // 再加 3 个新 entry
        for _ in 0..3 {
            tracker.record_evidence_read_at("session-1", now);
        }
        let count = tracker.count_recent_evidence_reads_at("session-1", now);
        assert_eq!(count, 3, "old entries beyond window should be excluded");
    }

    #[test]
    fn tracker_window_boundary_is_inclusive() {
        let tracker = RecentToolCallTracker::new();
        let now = Instant::now();
        // entry 在 exactly 60s 前 → 仍在 window 内 (`<= window`)
        let boundary = now - EVIDENCE_READ_WINDOW;
        tracker.record_evidence_read_at("session-1", boundary);
        let count = tracker.count_recent_evidence_reads_at("session-1", now);
        assert_eq!(count, 1);
    }

    #[test]
    fn forget_session_clears_tracking() {
        let tracker = RecentToolCallTracker::new();
        for _ in 0..30 {
            tracker.record_evidence_read("session-1");
        }
        assert_eq!(tracker.tracked_session_count(), 1);
        tracker.forget_session("session-1");
        assert_eq!(tracker.count_recent_evidence_reads("session-1"), 0);
        assert_eq!(tracker.tracked_session_count(), 0);
    }

    #[test]
    fn tracker_handles_many_sessions_independently() {
        let tracker = RecentToolCallTracker::new();
        for session_id in &["s1", "s2", "s3"] {
            for _ in 0..100 {
                tracker.record_evidence_read(session_id);
            }
        }
        assert_eq!(tracker.tracked_session_count(), 3);
        for session_id in &["s1", "s2", "s3"] {
            let call = proposal("evidence_read", session_id);
            assert!(classify_tool_call(&call, &tracker).is_some());
        }
        // 清一个不影响其他
        tracker.forget_session("s2");
        assert_eq!(tracker.tracked_session_count(), 2);
        assert!(classify_tool_call(&proposal("evidence_read", "s1"), &tracker).is_some());
        assert!(classify_tool_call(&proposal("evidence_read", "s2"), &tracker).is_none());
    }

    #[test]
    fn record_evidence_read_prunes_inline() {
        let tracker = RecentToolCallTracker::new();
        let old = Instant::now() - Duration::from_secs(70);
        for _ in 0..10 {
            tracker.record_evidence_read_at("s1", old);
        }
        // 新增一条 NOW → 内部 prune 顺手清掉所有 old entries
        tracker.record_evidence_read("s1");
        assert_eq!(tracker.count_recent_evidence_reads("s1"), 1);
    }

    #[test]
    fn threshold_and_window_constants_match_doc2() {
        // Doc 2 §5 明示 > 50 / min 触发警告
        assert_eq!(EVIDENCE_READ_THRESHOLD_PER_MINUTE, 50);
        assert_eq!(EVIDENCE_READ_WINDOW, Duration::from_secs(60));
    }
}

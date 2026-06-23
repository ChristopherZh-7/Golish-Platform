//! Harness 运行时灰度开关（feature flags）。
//!
//! 集中存放 env 驱动的开关，便于灰度发布、回滚与单测。读取逻辑抽成纯函数
//! `*_from_env`，这样测试不必改动真实进程环境即可覆盖各取值分支。

/// scoping 人工确认硬门禁灰度开关（设计 2026-06-06-scoping-per-mode-gate-hitl §6 R1）。
///
/// 默认开启；设 `GOLISH_SCOPING_HUMAN_GATE=0`（或 `false`，大小写不敏感）可关闭，
/// 回退到「scoping 无人工确认硬门禁」的旧行为。
pub fn scoping_human_gate_enabled() -> bool {
    scoping_human_gate_from_env(std::env::var("GOLISH_SCOPING_HUMAN_GATE").ok())
}

/// 纯函数：把环境变量取值映射为开关布尔，便于单测（不触碰真实进程环境）。
///
/// 缺省（未设）= 开启；`"0"` 或 `"false"`（大小写不敏感）= 关闭；其余非空值 = 开启。
fn scoping_human_gate_from_env(value: Option<String>) -> bool {
    match value {
        Some(v) => v != "0" && !v.eq_ignore_ascii_case("false"),
        None => true,
    }
}

/// submit 预检「完整 authoritative 口径」灰度开关（设计
/// `2026-06-23-submit-preview-authoritative-context.md` · T3）。
///
/// 默认开启；设 `GOLISH_SUBMIT_PREVIEW_AUTHORITATIVE_CONTEXT=0`（或 `false`，大小写
/// 不敏感）可关闭，回退到「submit 预检只喂 in_scope_assets + evidence_facts，不喂
/// asset_types / expected_techniques」的旧行为。开启时预检与 stage-close 同口径。
pub fn submit_preview_authoritative_context_enabled() -> bool {
    submit_preview_authoritative_context_from_env(
        std::env::var("GOLISH_SUBMIT_PREVIEW_AUTHORITATIVE_CONTEXT").ok(),
    )
}

/// 纯函数：环境变量取值 → 开关布尔（同 [`scoping_human_gate_from_env`] 语义，便于单测）。
fn submit_preview_authoritative_context_from_env(value: Option<String>) -> bool {
    match value {
        Some(v) => v != "0" && !v.eq_ignore_ascii_case("false"),
        None => true,
    }
}

/// failure ≠ checked_empty 灰度开关（T2，设计
/// `2026-06-23-failure-outcome-not-checked-empty.md`）。
///
/// **默认关闭**（opt-in）；设 `GOLISH_FAILURE_OUTCOME_ERROR=1`（或 `true`，大小写不
/// 敏感）开启——失败的被动检查（非零退出 / 超时 / 502）记 `error`（≠ `empty`），
/// gate 仍当终态但语义为「失败阻断」。缺省 = 失败记 `empty`（旧行为，逐字节不变）。
pub fn failure_outcome_error_enabled() -> bool {
    failure_outcome_error_from_env(std::env::var("GOLISH_FAILURE_OUTCOME_ERROR").ok())
}

/// 纯函数：缺省（未设）= 关闭；`"1"` / `"true"`（大小写不敏感）= 开启；其余 = 关闭。
fn failure_outcome_error_from_env(value: Option<String>) -> bool {
    match value {
        Some(v) => v == "1" || v.eq_ignore_ascii_case("true"),
        None => false,
    }
}

// 注：#4 technique_outcomes（写 + 读）/ #5 source_query_log（写）/ #6 expansion_queue
// （写）三组物化表路径**均无灰度开关、始终开启**（用户 2026-06-23 决定测试阶段默认开，
// 删除原 GOLISH_TECHNIQUE_OUTCOMES_WRITE / _READ / GOLISH_SOURCE_QUERY_LOG_WRITE /
// GOLISH_EXPANSION_QUEUE_WRITE 四个开关）。写路径非致命 warn（写失败 / 表未 apply 只
// warn，绝不影响主流程）；#4 读路径是 coverage gate 的 dual-read union（additive +
// fail-safe 到空：读失败 / 表缺即退回 ledger + coverage_truth）。各落库 / gate 点直接
// 无条件执行，不再查 env。

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_on_when_unset() {
        assert!(scoping_human_gate_from_env(None));
    }

    #[test]
    fn off_for_zero_and_false() {
        assert!(!scoping_human_gate_from_env(Some("0".to_string())));
        assert!(!scoping_human_gate_from_env(Some("false".to_string())));
        assert!(!scoping_human_gate_from_env(Some("FALSE".to_string())));
        assert!(!scoping_human_gate_from_env(Some("False".to_string())));
    }

    #[test]
    fn on_for_other_values() {
        assert!(scoping_human_gate_from_env(Some("1".to_string())));
        assert!(scoping_human_gate_from_env(Some("true".to_string())));
        assert!(scoping_human_gate_from_env(Some("yes".to_string())));
    }

    #[test]
    fn submit_preview_authoritative_defaults_on_when_unset() {
        assert!(submit_preview_authoritative_context_from_env(None));
    }

    #[test]
    fn submit_preview_authoritative_off_for_zero_and_false() {
        assert!(!submit_preview_authoritative_context_from_env(Some(
            "0".to_string()
        )));
        assert!(!submit_preview_authoritative_context_from_env(Some(
            "false".to_string()
        )));
        assert!(!submit_preview_authoritative_context_from_env(Some(
            "FALSE".to_string()
        )));
    }

    #[test]
    fn submit_preview_authoritative_on_for_other_values() {
        assert!(submit_preview_authoritative_context_from_env(Some(
            "1".to_string()
        )));
        assert!(submit_preview_authoritative_context_from_env(Some(
            "true".to_string()
        )));
    }

    #[test]
    fn failure_outcome_error_defaults_off_when_unset() {
        // opt-in：缺省关闭（失败仍记 empty，逐字节不变）。
        assert!(!failure_outcome_error_from_env(None));
    }

    #[test]
    fn failure_outcome_error_on_for_one_and_true() {
        assert!(failure_outcome_error_from_env(Some("1".to_string())));
        assert!(failure_outcome_error_from_env(Some("true".to_string())));
        assert!(failure_outcome_error_from_env(Some("TRUE".to_string())));
    }

    #[test]
    fn failure_outcome_error_off_for_zero_and_other_values() {
        assert!(!failure_outcome_error_from_env(Some("0".to_string())));
        assert!(!failure_outcome_error_from_env(Some("false".to_string())));
        assert!(!failure_outcome_error_from_env(Some("yes".to_string())));
    }
}

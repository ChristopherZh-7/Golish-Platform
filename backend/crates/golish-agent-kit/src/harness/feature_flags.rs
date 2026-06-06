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
}

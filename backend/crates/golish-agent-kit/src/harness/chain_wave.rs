//! Chain-wave controller (设计 2026-07-02-attack-stage-formulaic-candidate-exploit
//! §3.5). Pure, DB-free decision function for the `attack_candidate ⇄ verification`
//! candidate loop (漏洞联动 a→b→c).
//!
//! The DAG stays acyclic (`operation_graph.rs` Kahn-validates, no
//! `verification → attack_candidate` back-edge). The wave loop is expressed HERE
//! at the graph-flow layer: after a `verification` gate PASS, the controller asks
//! [`decide_chain_wave`]; on [`WaveDecision::OpenNextWave`] it overwrites the
//! cursor back to `attack_candidate` (a new wave) instead of taking the DAG's
//! topological next, and on [`WaveDecision::Advance`] it takes the normal
//! transition. All stopping is bounded (dedupe + fuel + depth) so the loop cannot
//! run away.

use std::collections::HashSet;

use super::types::AttackCandidate;

/// Outcome of the post-verification wave decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaveDecision {
    /// A new, untested hypothesis exists within the fuel/depth budget — overwrite
    /// the cursor back to `attack_candidate` for wave `next_wave`.
    OpenNextWave { next_wave: u32 },
    /// No new hypothesis, or a cap was hit — take the normal DAG transition
    /// (reporting / access_validation).
    Advance,
}

/// Normalized de-dup key for a candidate hypothesis.
///
/// Mirrors the `golish-db` `hypothesis_hash` normalization (trim + collapse
/// internal whitespace + lowercase over `target | technique | hypothesis`) but
/// returns the normalized string rather than its sha256, so this module stays
/// DB-free and dependency-free. Dedup semantics are equivalent (a wave's seen-set
/// and this wave's candidates are compared with the same key).
pub fn candidate_dedup_key(c: &AttackCandidate) -> String {
    let norm = |s: &str| {
        s.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    format!(
        "{}\u{1f}{}\u{1f}{}",
        norm(&c.target),
        norm(c.technique.as_deref().unwrap_or("")),
        norm(&c.hypothesis)
    )
}

/// Decide whether to open another candidate wave after a `verification` gate PASS.
///
/// * `spawned_this_wave` — the follow-on candidates this wave produced (proposed
///   hypotheses derived from verified/refuted findings, typically carrying a
///   `parent_finding_id`). Empty = the wave surfaced nothing new.
/// * `seen_hypothesis_keys` — de-dup keys ([`candidate_dedup_key`]) of every
///   hypothesis tested in prior waves.
/// * `current_wave` — the wave that just completed (0-based).
/// * `max_waves` — total wave fuel (tie-break against runaway loops).
/// * `max_chain_depth` — longest a→b→c chain in hops (measured in waves).
///
/// Returns [`WaveDecision::OpenNextWave`] iff there is at least one candidate
/// whose key is NOT in `seen_hypothesis_keys` AND the next wave is within both the
/// fuel and depth caps; otherwise [`WaveDecision::Advance`].
pub fn decide_chain_wave(
    spawned_this_wave: &[AttackCandidate],
    seen_hypothesis_keys: &HashSet<String>,
    current_wave: u32,
    max_waves: u32,
    max_chain_depth: u32,
) -> WaveDecision {
    let next_wave = current_wave.saturating_add(1);
    // Fuel cap: never open a wave beyond the total budget.
    if next_wave > max_waves {
        return WaveDecision::Advance;
    }
    // Depth cap: each wave is one a→b hop; the follow-ons would sit at
    // `next_wave`, so stop if that would exceed the configured chain depth.
    if next_wave > max_chain_depth {
        return WaveDecision::Advance;
    }
    let has_new = spawned_this_wave
        .iter()
        .any(|c| !seen_hypothesis_keys.contains(&candidate_dedup_key(c)));
    if has_new {
        WaveDecision::OpenNextWave { next_wave }
    } else {
        WaveDecision::Advance
    }
}

/// Default wave fuel (设计 §11 open-question 2 recommended default).
pub const DEFAULT_MAX_WAVES: u32 = 5;
/// Default chain depth (设计 §11 open-question 2 recommended default).
pub const DEFAULT_MAX_CHAIN_DEPTH: u32 = 3;

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn candidate(target: &str, technique: Option<&str>, hypothesis: &str) -> AttackCandidate {
        AttackCandidate {
            candidate_id: Uuid::new_v4(),
            target: target.to_string(),
            hypothesis: hypothesis.to_string(),
            technique: technique.map(str::to_string),
            rationale: "r".to_string(),
            evidence_refs: vec![],
            prior_refs: vec![],
            suggested_approach: String::new(),
            priority: super::super::types::CandidatePriority::Medium,
            wave: 0,
            parent_finding_id: None,
            disposition: super::super::types::CandidateDisposition::Verified,
        }
    }

    #[test]
    fn opens_next_wave_when_new_hypothesis_within_caps() {
        let spawned = vec![candidate("a", Some("T1"), "new hypothesis")];
        let seen = HashSet::new();
        assert_eq!(
            decide_chain_wave(&spawned, &seen, 0, 5, 3),
            WaveDecision::OpenNextWave { next_wave: 1 }
        );
    }

    #[test]
    fn advances_when_no_candidates() {
        let seen = HashSet::new();
        assert_eq!(
            decide_chain_wave(&[], &seen, 0, 5, 3),
            WaveDecision::Advance
        );
    }

    #[test]
    fn advances_when_over_wave_fuel() {
        // current_wave == max_waves → next wave would be max_waves+1 > cap.
        let spawned = vec![candidate("a", Some("T1"), "new")];
        let seen = HashSet::new();
        assert_eq!(
            decide_chain_wave(&spawned, &seen, 5, 5, 99),
            WaveDecision::Advance
        );
    }

    #[test]
    fn advances_when_hypothesis_already_seen() {
        let c = candidate("a", Some("T1"), "seen hypothesis");
        let mut seen = HashSet::new();
        seen.insert(candidate_dedup_key(&c));
        assert_eq!(
            decide_chain_wave(&[c], &seen, 0, 5, 3),
            WaveDecision::Advance
        );
    }

    #[test]
    fn advances_when_over_chain_depth() {
        // wave fuel is generous but the chain-depth cap is hit first.
        let spawned = vec![candidate("a", Some("T1"), "new")];
        let seen = HashSet::new();
        assert_eq!(
            decide_chain_wave(&spawned, &seen, 2, 99, 2),
            WaveDecision::Advance
        );
    }

    #[test]
    fn dedup_key_normalizes_whitespace_and_case() {
        let a = candidate("Api.Example.com", Some("WSTG-ATHZ-04"), "IDOR  on /x");
        let b = candidate("api.example.com", Some("wstg-athz-04"), "idor on /x");
        assert_eq!(candidate_dedup_key(&a), candidate_dedup_key(&b));
    }

    #[test]
    fn mixed_seen_and_new_opens_wave() {
        let old = candidate("a", Some("T1"), "old");
        let fresh = candidate("b", Some("T2"), "fresh");
        let mut seen = HashSet::new();
        seen.insert(candidate_dedup_key(&old));
        assert_eq!(
            decide_chain_wave(&[old, fresh], &seen, 1, 5, 3),
            WaveDecision::OpenNextWave { next_wave: 2 }
        );
    }
}

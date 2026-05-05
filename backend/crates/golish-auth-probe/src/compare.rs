//! Round comparison engine — turns 2-3 [`Round`]s into a [`Verdict`].
//!
//! Decision tables mirror `docs/auth-probe-contract.md` §3.

use crate::types::{Round, RoundOutcome, Scenario, Severity, Verdict};

/// Compare rounds for a (endpoint, scenario) tuple. Returns
/// (verdict, severity, diff_summary).
pub fn compare_rounds(
    scenario: Scenario,
    rounds: &[Option<&Round>],
) -> (Verdict, Severity, String) {
    match scenario {
        Scenario::Anonymous => compare_anonymous(rounds),
        Scenario::CrossUser => compare_cross_user(rounds),
        Scenario::Privilege => compare_privilege(rounds),
    }
}

fn compare_anonymous(rounds: &[Option<&Round>]) -> (Verdict, Severity, String) {
    let r1 = match rounds.first().and_then(|r| *r) {
        Some(r) => r,
        None => {
            return (
                Verdict::Error,
                Severity::Info,
                "round 1 (anonymous) missing".into(),
            )
        }
    };
    let r2 = rounds.get(1).and_then(|r| *r);

    if r1.outcome == RoundOutcome::NetworkError {
        return (
            Verdict::Error,
            Severity::Info,
            "anonymous request had a network error".into(),
        );
    }

    match r1.outcome {
        RoundOutcome::Success => match r2 {
            Some(r) if r.outcome == RoundOutcome::Success => {
                let body_match = body_similarity(r1, r) >= 0.8;
                if r1.body_len == r.body_len && r1.snippet == r.snippet {
                    (
                        Verdict::Vulnerable,
                        Severity::Critical,
                        "anonymous and authenticated bodies are byte-for-byte identical".into(),
                    )
                } else if body_match {
                    (
                        Verdict::Vulnerable,
                        Severity::High,
                        "anonymous and authenticated bodies are >=80% similar".into(),
                    )
                } else {
                    (
                        Verdict::Potential,
                        Severity::Medium,
                        "anonymous returned 2xx but body differs from authenticated".into(),
                    )
                }
            }
            Some(_) => (
                Verdict::Vulnerable,
                Severity::High,
                "anonymous succeeded while authenticated did not — server-side bug".into(),
            ),
            None => (
                Verdict::Potential,
                Severity::Medium,
                "anonymous returned 2xx, no authenticated baseline available".into(),
            ),
        },
        RoundOutcome::AuthDenied => (
            Verdict::NotVulnerable,
            Severity::Info,
            format!("anonymous correctly denied with HTTP {}", r1.status),
        ),
        RoundOutcome::RateLimited => (
            Verdict::Inconclusive,
            Severity::Info,
            "rate-limited during anonymous probe".into(),
        ),
        RoundOutcome::ServerError => (
            Verdict::Inconclusive,
            Severity::Info,
            format!("anonymous probe got HTTP {}", r1.status),
        ),
        _ => (
            Verdict::Inconclusive,
            Severity::Info,
            format!("anonymous probe got unexpected outcome {:?}", r1.outcome),
        ),
    }
}

fn compare_cross_user(rounds: &[Option<&Round>]) -> (Verdict, Severity, String) {
    let r1 = match rounds.first().and_then(|r| *r) {
        Some(r) => r,
        None => return (Verdict::Error, Severity::Info, "round 1 missing".into()),
    };
    let r2 = match rounds.get(1).and_then(|r| *r) {
        Some(r) => r,
        None => return (Verdict::Error, Severity::Info, "round 2 missing".into()),
    };
    let r3 = rounds.get(2).and_then(|r| *r);

    // Round 1 = token A on A's resource (baseline)
    // Round 2 = token A on B's resource (the IDOR test)
    // Round 3 = token B on B's resource (sanity baseline)
    if r1.outcome != RoundOutcome::Success {
        return (
            Verdict::Inconclusive,
            Severity::Info,
            format!(
                "cross-user baseline (token A + A's id) failed: HTTP {}",
                r1.status
            ),
        );
    }
    match r2.outcome {
        RoundOutcome::Success => {
            let r3_says_legitimately_owned = matches!(
                r3.map(|r| r.outcome),
                Some(RoundOutcome::Success)
            );
            if r3_says_legitimately_owned {
                (
                    Verdict::Vulnerable,
                    Severity::High,
                    "token A successfully accessed user B's resource (IDOR)".into(),
                )
            } else {
                (
                    Verdict::Potential,
                    Severity::Medium,
                    "token A reached user B's resource but B's own access is uncertain".into(),
                )
            }
        }
        RoundOutcome::AuthDenied | RoundOutcome::NotFound => (
            Verdict::NotVulnerable,
            Severity::Info,
            format!(
                "token A correctly denied on user B's resource (HTTP {})",
                r2.status
            ),
        ),
        _ => (
            Verdict::Inconclusive,
            Severity::Info,
            format!("cross-user attempt got HTTP {}", r2.status),
        ),
    }
}

fn compare_privilege(rounds: &[Option<&Round>]) -> (Verdict, Severity, String) {
    let r1 = match rounds.first().and_then(|r| *r) {
        Some(r) => r,
        None => return (Verdict::Error, Severity::Info, "round 1 missing".into()),
    };
    let r2 = rounds.get(1).and_then(|r| *r);

    match r1.outcome {
        RoundOutcome::Success => (
            Verdict::Vulnerable,
            Severity::High,
            "low-privilege token reached an admin-shaped endpoint".into(),
        ),
        RoundOutcome::AuthDenied => match r2 {
            Some(r) if r.outcome == RoundOutcome::AuthDenied => (
                Verdict::Inconclusive,
                Severity::Info,
                "endpoint denies both authed and anonymous — needs a higher-priv token to exercise".into(),
            ),
            _ => (
                Verdict::NotVulnerable,
                Severity::Info,
                format!(
                    "low-privilege token correctly denied (HTTP {})",
                    r1.status
                ),
            ),
        },
        _ => (
            Verdict::Inconclusive,
            Severity::Info,
            format!("privilege probe got HTTP {}", r1.status),
        ),
    }
}

/// Cheap similarity score over `(body_len, snippet)`. We avoid full
/// edit-distance because the snippet is capped at 200 bytes and we
/// only care about coarse "same-ish vs different".
fn body_similarity(a: &Round, b: &Round) -> f32 {
    if a.body_len == 0 && b.body_len == 0 {
        return 1.0;
    }
    let len_min = a.body_len.min(b.body_len) as f32;
    let len_max = a.body_len.max(b.body_len) as f32;
    let len_score = if len_max > 0.0 {
        len_min / len_max
    } else {
        0.0
    };
    let snippet_score = if a.snippet == b.snippet {
        1.0
    } else if !a.snippet.is_empty()
        && a.snippet.len() == b.snippet.len()
        && a.snippet
            .chars()
            .zip(b.snippet.chars())
            .filter(|(x, y)| x == y)
            .count() as f32
            / a.snippet.chars().count() as f32
            >= 0.5
    {
        // Same length and >=50% char overlap — likely the same shape.
        0.7
    } else {
        0.0
    };
    (len_score * 0.6) + (snippet_score * 0.4)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round(status: u16, outcome: RoundOutcome, body: &str) -> Round {
        Round {
            status,
            body_len: body.len(),
            snippet: body.chars().take(200).collect(),
            outcome,
            retry_after_secs: None,
        }
    }

    #[test]
    fn anonymous_denied_then_success_is_not_vulnerable() {
        let r1 = round(401, RoundOutcome::AuthDenied, "");
        let r2 = round(200, RoundOutcome::Success, "ok");
        let (v, s, _) = compare_rounds(Scenario::Anonymous, &[Some(&r1), Some(&r2)]);
        assert_eq!(v, Verdict::NotVulnerable);
        assert_eq!(s, Severity::Info);
    }

    #[test]
    fn anonymous_identical_bodies_critical() {
        let r1 = round(200, RoundOutcome::Success, "[\"user1\",\"user2\"]");
        let r2 = round(200, RoundOutcome::Success, "[\"user1\",\"user2\"]");
        let (v, s, _) = compare_rounds(Scenario::Anonymous, &[Some(&r1), Some(&r2)]);
        assert_eq!(v, Verdict::Vulnerable);
        assert_eq!(s, Severity::Critical);
    }

    #[test]
    fn cross_user_token_a_reaches_b_high() {
        let r1 = round(200, RoundOutcome::Success, "{\"id\":1}");
        let r2 = round(200, RoundOutcome::Success, "{\"id\":2}");
        let r3 = round(200, RoundOutcome::Success, "{\"id\":2}");
        let (v, s, _) = compare_rounds(
            Scenario::CrossUser,
            &[Some(&r1), Some(&r2), Some(&r3)],
        );
        assert_eq!(v, Verdict::Vulnerable);
        assert_eq!(s, Severity::High);
    }

    #[test]
    fn cross_user_403_on_b_is_not_vulnerable() {
        let r1 = round(200, RoundOutcome::Success, "{}");
        let r2 = round(403, RoundOutcome::AuthDenied, "");
        let (v, _, _) =
            compare_rounds(Scenario::CrossUser, &[Some(&r1), Some(&r2), None]);
        assert_eq!(v, Verdict::NotVulnerable);
    }

    #[test]
    fn privilege_low_priv_succeeded_high() {
        let r1 = round(200, RoundOutcome::Success, "{\"users\":[]}");
        let (v, s, _) = compare_rounds(Scenario::Privilege, &[Some(&r1), None]);
        assert_eq!(v, Verdict::Vulnerable);
        assert_eq!(s, Severity::High);
    }
}

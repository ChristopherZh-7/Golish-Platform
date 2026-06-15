# Host-aware coverage (Phase 2a) — implementation plan

> **For the AI worker:** execute task-by-task with `.cursor/skills/executing-plans`
> (TDD, frequent commits). Spec: `docs/design/2026-06-15-host-aware-coverage.md`
> (§4.0 is the chosen 2a approach). This plan covers **Phase 2a only**
> (`target_intel`, drop-only, no new collectors); 2b/2c are sketched at the end.

**Goal:** Make the `target_intel` coverage gate stop demanding domain-only
techniques (`SUBDOMAIN` / forward `DNS` / `CT`) from IP/CIDR assets, so an
in-scope IP is no longer permanently "incomplete".
**Architecture:** A per-stage `StageSpec.host_aware_coverage` flag (default off,
mirrors `facts_from_db_truth`). When on, `coverage_complete` filters each
in-scope asset's expected techniques by the asset's class, inferred from its
**value** (`AssetClass::from_value`) against a small `technique_applies` matrix.
No `GateContext`/hook/DB change. Gate stays a pure function.
**Stack:** Rust (`golish-agent-kit`), JSON stage spec.

## Decisions locked (from design §8 + user "按默认")

- **2a only** = `target_intel`. EAS/enumeration (2b), truth-projection + IP-native
  techniques (2c) are deferred.
- **Drop-only**: for IP/CIDR, *remove* the domain-only techniques; do **not** add
  new IP-native techniques/collectors (2c).
- **URL inherits host** (domain) techniques minus subdomain enumeration.
- **Classify from value** (no `targets.type` on the axis): IP if it parses as
  `IpAddr`, URL if `http(s)://`, CIDR if `addr/prefix`, else Domain. `Other` is
  the fail-safe (keeps the full list).
- Flag stays **off in JSON until Task A.4's parity test is green**.

## File structure

- `golish-agent-kit/src/harness/technique_resolver.rs` — add
  `AssetClass::from_value`, `technique_applies`, `techniques_for`; pure, unit-tested.
- `golish-agent-kit/src/harness/stage_spec.rs` — add `host_aware_coverage: bool`
  flag (+ parse test); fix any full `StageSpec { .. }` literals.
- `golish-agent-kit/src/harness/gate/rule_engine.rs` — `coverage_complete` inner
  loop filters per asset when the flag is on (+ parity test).
- `resources/harness/stages/target_intel.json` — set the flag true (last).

---

## Task A.1 — per-asset-type matrix in `technique_resolver` (TDD)

**File:** `backend/crates/golish-agent-kit/src/harness/technique_resolver.rs`

### Step 1 — write the failing tests (append to the existing `mod tests`)

```rust
    #[test]
    fn from_value_classifies_ip_url_cidr_domain() {
        assert_eq!(AssetClass::from_value("1.2.3.4"), AssetClass::Ip);
        assert_eq!(AssetClass::from_value("2606:4700::1111"), AssetClass::Ip);
        assert_eq!(AssetClass::from_value("https://a.com/x"), AssetClass::Url);
        assert_eq!(AssetClass::from_value("10.0.0.0/24"), AssetClass::Cidr);
        assert_eq!(AssetClass::from_value("a.example.com"), AssetClass::Domain);
        assert_eq!(AssetClass::from_value(""), AssetClass::Other);
    }

    #[test]
    fn target_intel_drops_domain_only_techniques_for_ip() {
        let ip = techniques_for(StageKind::TargetIntel, AssetClass::Ip);
        assert!(!ip.contains(&"GOLISH-INTEL-SUBDOMAIN".to_string()));
        assert!(!ip.contains(&"GOLISH-INTEL-DNS".to_string()));
        assert!(!ip.contains(&"GOLISH-INTEL-CT".to_string()));
        assert!(ip.contains(&"GOLISH-INTEL-WHOIS".to_string()));
        assert!(ip.contains(&"GOLISH-INTEL-ASN".to_string()));
        assert!(ip.contains(&"GOLISH-INTEL-OSINT".to_string()));
        // CIDR matches IP.
        assert_eq!(
            techniques_for(StageKind::TargetIntel, AssetClass::Cidr),
            techniques_for(StageKind::TargetIntel, AssetClass::Ip)
        );
        // Domain keeps all 6.
        assert_eq!(techniques_for(StageKind::TargetIntel, AssetClass::Domain).len(), 6);
        // URL keeps host intel (DNS/CT) but not subdomain enumeration.
        let url = techniques_for(StageKind::TargetIntel, AssetClass::Url);
        assert!(!url.contains(&"GOLISH-INTEL-SUBDOMAIN".to_string()));
        assert!(url.contains(&"GOLISH-INTEL-DNS".to_string()));
        assert!(url.contains(&"GOLISH-INTEL-CT".to_string()));
    }

    #[test]
    fn other_class_keeps_full_set_failsafe() {
        assert_eq!(techniques_for(StageKind::TargetIntel, AssetClass::Other).len(), 6);
    }

    #[test]
    fn non_target_intel_stage_keeps_all_techniques_in_2a() {
        // 2a only differentiates target_intel; EAS keeps its 3 for every class.
        assert_eq!(
            techniques_for(StageKind::ExternalAttackSurface, AssetClass::Ip).len(),
            techniques_for(StageKind::ExternalAttackSurface, AssetClass::Domain).len()
        );
    }
```

### Step 2 — run, confirm failure

```bash
cd backend && cargo nextest run -p golish-agent-kit technique_resolver
```
Expect: compile error / failures (`from_value`, `technique_applies`,
`techniques_for` undefined).

### Step 3 — implement

Add to `impl AssetClass` (next to `from_target_type`):

```rust
    /// Infer the asset class from a target **value** string (the form carried in
    /// `GateContext.in_scope_assets`). Lets host-aware coverage classify without
    /// an authoritative `targets.type` on the axis. Conservative: an
    /// unrecognized value falls through to `Domain` (the strict, full-technique
    /// set for intel), and empty → `Other` — neither relaxes the gate.
    pub fn from_value(value: &str) -> Self {
        let v = value.trim();
        if v.is_empty() {
            return Self::Other;
        }
        let lower = v.to_ascii_lowercase();
        if lower.starts_with("http://") || lower.starts_with("https://") {
            return Self::Url;
        }
        if v.parse::<std::net::IpAddr>().is_ok() {
            return Self::Ip;
        }
        if let Some((addr, prefix)) = v.split_once('/') {
            if addr.parse::<std::net::IpAddr>().is_ok() && prefix.parse::<u8>().is_ok() {
                return Self::Cidr;
            }
        }
        Self::Domain
    }
```

Add at module level (after `resolve_expected_techniques`):

```rust
/// Host-aware coverage (design 2026-06-15 §3): whether `tech` (one of `stage`'s
/// baseline techniques) applies to a single asset of `class`. Phase 2a only
/// differentiates `TargetIntel`; other stages return `true` (no-op until 2b).
/// `Other` keeps every technique (fail-safe: an unclassified asset is never
/// under-checked).
pub fn technique_applies(stage: StageKind, class: AssetClass, tech: &str) -> bool {
    use AssetClass::*;
    if matches!(class, Other) {
        return true;
    }
    match stage {
        StageKind::TargetIntel => match tech {
            // Subdomain enumeration only makes sense for a domain.
            "GOLISH-INTEL-SUBDOMAIN" => matches!(class, Domain),
            // Forward DNS + cert transparency are domain/host concepts; a bare
            // IP/CIDR has neither a self-keyed forward A record nor a CT log.
            "GOLISH-INTEL-DNS" | "GOLISH-INTEL-CT" => matches!(class, Domain | Url),
            // WHOIS / ASN / OSINT apply to every class (org/netblock-wide).
            _ => true,
        },
        // 2b: EAS / enumeration matrices land here.
        _ => true,
    }
}

/// Convenience: the subset of `stage`'s baseline that applies to `class`
/// (= `technique_applies` over `stage_baseline`). For tests/diagnostics.
pub fn techniques_for(stage: StageKind, class: AssetClass) -> Vec<String> {
    stage_baseline(stage)
        .into_iter()
        .filter(|t| technique_applies(stage, class, t))
        .map(String::from)
        .collect()
}
```

### Step 4 — run, confirm pass

```bash
cd backend && cargo nextest run -p golish-agent-kit technique_resolver
```
Expect: all green.

### Step 5 — commit

```bash
git add backend/crates/golish-agent-kit/src/harness/technique_resolver.rs
git commit -m "feat(harness): per-asset-type technique matrix (host-aware coverage 2a)"
```

---

## Task A.2 — `StageSpec.host_aware_coverage` flag (TDD)

**File:** `backend/crates/golish-agent-kit/src/harness/stage_spec.rs`

### Step 1 — failing parse test (append to `mod tests`)

```rust
    #[test]
    fn host_aware_coverage_defaults_false_and_parses() {
        let minimal = r#"{"id":"target_intel","kind":"target_intel","risk_level":"low",
            "deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate"}"#;
        assert!(!load_stage_spec_from_json(minimal).expect("parse").host_aware_coverage);
        let on = r#"{"id":"target_intel","kind":"target_intel","risk_level":"low",
            "deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate",
            "host_aware_coverage":true}"#;
        assert!(load_stage_spec_from_json(on).expect("parse").host_aware_coverage);
    }
```

### Step 2 — run, confirm failure

```bash
cd backend && cargo nextest run -p golish-agent-kit stage_spec
```
Expect: compile error (`host_aware_coverage` is not a field).

### Step 3 — add the field (right after `facts_from_db_truth`)

```rust
    /// Host-aware coverage (design 2026-06-15): when true, `coverage_complete`
    /// filters each in-scope asset's expected techniques by the asset's class
    /// (a bare IP is not asked for SUBDOMAIN/DNS/CT). Default false =
    /// byte-for-byte unchanged. Enable only after a green PASS/BLOCK parity test.
    #[serde(default)]
    pub host_aware_coverage: bool,
```

### Step 4 — fix full `StageSpec { .. }` literals

```bash
cd backend && rg -n "StageSpec \{" crates/golish-agent-kit/src
```
For each **full** struct literal (not `..Default::default()`), add
`host_aware_coverage: false,`. Known one: `gate/finding_verification_check.rs`
`fn spec_with` (after `facts_from_db_truth: false,`). Apply to any others rg finds.

### Step 5 — run, confirm pass

```bash
cd backend && cargo nextest run -p golish-agent-kit stage_spec && cargo check -p golish-agent-kit
```
Expect: green.

### Step 6 — commit

```bash
git add backend/crates/golish-agent-kit/src/harness/stage_spec.rs backend/crates/golish-agent-kit/src/harness/gate/finding_verification_check.rs
git commit -m "feat(harness): StageSpec.host_aware_coverage flag (default off)"
```

---

## Task A.3 — `coverage_complete` filters per asset + parity test (TDD)

**File:** `backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs`

### Step 1 — failing parity test (append to `mod tests`, reuse existing helpers)

Reuse the module's existing coverage-test scaffolding (`spec_with_expected`,
the `StageDeliverable` builder used by the other `coverage_complete` tests, and
`EvidenceFact`/`EvidenceOutcome`). Add:

```rust
    #[test]
    fn host_aware_coverage_relaxes_ip_not_domain() {
        use super::super::types::StageKind;
        let techs = [
            "GOLISH-INTEL-DNS", "GOLISH-INTEL-SUBDOMAIN", "GOLISH-INTEL-CT",
            "GOLISH-INTEL-WHOIS", "GOLISH-INTEL-ASN", "GOLISH-INTEL-OSINT",
        ];
        // domain satisfies all 6; IP satisfies only WHOIS/ASN/OSINT.
        let mut facts: Vec<EvidenceFact> = techs
            .iter()
            .map(|t| EvidenceFact {
                asset: "a.com".into(),
                technique: (*t).into(),
                outcome: EvidenceOutcome::Found,
            })
            .collect();
        for t in ["GOLISH-INTEL-WHOIS", "GOLISH-INTEL-ASN", "GOLISH-INTEL-OSINT"] {
            facts.push(EvidenceFact {
                asset: "1.2.3.4".into(),
                technique: t.into(),
                outcome: EvidenceOutcome::Found,
            });
        }
        let ctx = GateContext {
            in_scope_assets: Some(vec!["a.com".into(), "1.2.3.4".into()]),
            expected_techniques: Some(techs.iter().map(|s| s.to_string()).collect()),
            evidence_facts: Some(facts),
        };
        // Build a deliverable with NO self-reported coverage (facts-driven), and
        // a target_intel spec; mirror the existing coverage_complete tests for the
        // deliverable/on_fail construction.
        let d = coverage_test_deliverable_empty(); // existing helper / inline builder
        let mut spec = spec_with_expected(&techs);
        spec.kind = StageKind::TargetIntel;

        // derive_from_evidence=true so Found facts fill cells; authoritative off.
        let run = |spec: &StageSpec| {
            coverage_complete(&d, spec, &ctx, None, false, true, false, None, &default_on_fail())
        };

        // Flag OFF: IP is missing SUBDOMAIN/DNS/CT → Block.
        spec.host_aware_coverage = false;
        assert!(matches!(run(&spec), GateCheckOutcome::Block { .. }));

        // Flag ON: IP only needs WHOIS/ASN/OSINT (all Found) → Pass; domain
        // unchanged (still has all 6). The ONLY delta is the IP's 3 dropped cells.
        spec.host_aware_coverage = true;
        assert!(matches!(run(&spec), GateCheckOutcome::Pass));
    }
```

> If `coverage_test_deliverable_empty` / `default_on_fail` aren't already present,
> copy the exact deliverable + `OnFail` construction from the nearest existing
> `coverage_complete` test in this module (do not invent new shapes).

### Step 2 — run, confirm failure

```bash
cd backend && cargo nextest run -p golish-agent-kit host_aware_coverage_relaxes
```
Expect: the flag-ON case fails (currently the IP is still held to all 6 → Block).

### Step 3 — implement the per-asset filter

In `coverage_complete`, replace the gap double-loop preamble:

```rust
    let mut gaps: Vec<String> = Vec::new();
    for asset in &assets {
        for tech in techniques {
```

with:

```rust
    let mut gaps: Vec<String> = Vec::new();
    for asset in &assets {
        // Host-aware coverage (design 2026-06-15 §4.0): when enabled, hold each
        // asset only to the techniques that apply to its class (a bare IP isn't
        // asked for SUBDOMAIN/DNS/CT). Flag off ⇒ full `techniques` for every
        // asset (byte-identical to before). `asset: &&str` deref-coerces to &str.
        let asset_techniques: Vec<&String> = if spec.host_aware_coverage {
            let class = crate::harness::technique_resolver::AssetClass::from_value(asset);
            techniques
                .iter()
                .filter(|t| {
                    crate::harness::technique_resolver::technique_applies(spec.kind, class, t)
                })
                .collect()
        } else {
            techniques.iter().collect()
        };
        for tech in asset_techniques {
```

(The loop body is unchanged; `tech` is now `&&String` — the existing comparisons
use `*tech`/`tech.as_str()` which still work, but confirm with the compiler and
add a single deref if needed.)

### Step 4 — run, confirm pass + no regression

```bash
cd backend && cargo nextest run -p golish-agent-kit coverage && cargo nextest run -p golish-agent-kit
```
Expect: the new parity test + all existing harness tests green.

### Step 5 — commit

```bash
git add backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs
git commit -m "feat(harness): coverage_complete holds each asset to its type's techniques"
```

---

## Task A.4 — enable on `target_intel` + full verification

**File:** `resources/harness/stages/target_intel.json`

### Step 1 — set the flag

Add the top-level key (sibling of `expected_techniques`):

```json
  "host_aware_coverage": true,
```

### Step 2 — verify

```bash
python3 -m json.tool resources/harness/stages/target_intel.json >/dev/null && echo JSON_OK
cd backend && cargo nextest run -p golish-agent-kit && cargo check -p golish
```
Expect: `JSON_OK`; all `golish-agent-kit` tests green; `golish` checks.
Then full gate: `just precommit` (capture the result into `agent-progress.md`).

### Step 3 — record + status

- `agent-progress.md`: new session record with the commands + outputs above.
- `feature_list.json` `host-aware-coverage-2026-06-15`: 2a → `passing` (note 2b/2c
  still `not_started`), fill `evidence` with the precommit result.

### Step 4 — commit

```bash
git add resources/harness/stages/target_intel.json agent-progress.md feature_list.json
git commit -m "feat(harness): enable host-aware coverage on target_intel (2a)"
```

---

## Phase 2b — EAS + enumeration (deferred, sketch)

Extend `technique_applies` with `ExternalAttackSurface` (LIVENESS/PORT/
SERVICE-FP = host-level; domains via resolved host) and `Enumeration`
(DIR/PARAM/JSAPI = web/URL-level; formalizes the existing scope-level PARAM
rule per-asset). Same flag, same parity-test discipline; set the flag on
`external_attack_surface.json` / `enumeration.json`.

## Phase 2c — full host model (deferred, sketch)

Authoritative `targets.type` on the axis (design §4.1–4.3: typed
`GateContext`/hook/DB read) + IP-native techniques (reverse-DNS/PTR, IP-WHOIS)
with new `coverage_truth` queries + collectors. Largest; its own spec+plan.

---

## Self-check (writing-plans)

- **Spec coverage:** design §3.1 matrix ↔ A.1 `technique_applies`; §4.0 chosen
  approach ↔ A.1–A.3; §6 flag + parity ↔ A.2 (flag) + A.3 (parity test) + A.4
  (enable last); §5 phasing ↔ 2a tasks + 2b/2c sketches.
- **Type consistency:** `AssetClass::from_value`, `technique_applies(stage,
  class, tech)`, `techniques_for(stage, class)`, `StageSpec.host_aware_coverage`,
  `spec.kind` used identically across A.1–A.4.
- **No placeholders:** every code step has real code; the only "reuse existing"
  notes are for **test scaffolding** (deliverable/on_fail builders) that already
  exist in the module — not production code.

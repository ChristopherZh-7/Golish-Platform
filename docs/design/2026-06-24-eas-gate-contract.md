# EAS gate contract tightening

> Status: proposed + first implementation slice, 2026-06-24.
> Related: `docs/design/2026-06-24-intel-to-eas-handoff.md`, `resources/harness/stages/external_attack_surface/spec.json`.

## 1. Problem

`external_attack_surface` is the first active stage. Its job is not to say "we looked around"; it must define the concrete surface that later stages can test:

- which hosts or URLs are live;
- which concrete hosts/IPs have which open ports;
- which open ports have service/version fingerprints;
- which targets were checked and empty, blocked, or not applicable.

The current stage already has `coverage_complete`, host-aware coverage, and a freshness window. That is a good base, but the gate is still too tolerant in three places:

1. A self-reported `found` or `checked_empty` coverage cell can exist without cell-level evidence.
2. `blocked` / `not_applicable` can be empty labels with no explanation.
3. The "every open port must be fingerprinted" rule is a prompt habit, not a denominator contract.

This design tightens those three points without changing the database schema or enabling `authoritative_found` for active stages. The active Empty fact source is still not ready, so EAS remains `derive_from_evidence=true` but not authoritative.

## 2. Stage contract

EAS coverage is asset-type aware:

| Asset type | Required EAS outcome |
|---|---|
| `ip` | Port scan is mandatory. Liveness may be proven by an open-port scan result. Every discovered open port must be service-fingerprinted or explicitly blocked with evidence/note. |
| `domain` | Liveness and port scan are mandatory. Every discovered open port must be service-fingerprinted. |
| `url` | URL liveness is mandatory. Port and service coverage belong to the host/IP asset, not the path URL cell. Existing host-aware coverage drops PORT/SERVICE-FINGERPRINT for bare URL assets. |
| `cidr` | The CIDR itself is a range, not a host. Sweep/active scan is gated by approval. Discovered live IPs must be registered as concrete child targets and then satisfy the IP rule. No live hosts is `checked_empty` with sweep evidence. |
| `wildcard` | The wildcard is not actively brute-forced here. Concrete hosts inherited from `target_intel` must satisfy the domain/IP rules; the wildcard cell itself is `not_applicable` with a note. |

## 3. Gate rules for this slice

This slice uses existing data-driven gate ops:

1. `for_all coverage where status=found require evidence_refs`
2. `for_all coverage where status=checked_empty require evidence_refs`
3. `coverage_complete(require_note_for_other=true)`
4. `coverage_denominator(min_sample_ratio_pct=100)`

The prober prompt must tell the agent how to populate denominator fields:

- liveness success: `tested_units=1`, `total_units=1`;
- port scan success or empty: `tested_units=<ports attempted or scanned set size>`, `total_units=<same denominator>`;
- service fingerprint: `tested_units=<open ports fingerprinted>`, `total_units=<open ports discovered>`;
- no open ports: service fingerprint is `not_applicable` with a note, not `checked_empty total_units=0`;
- partial service fingerprint is blocked unless it has a real sampling rationale, which should be rare for EAS.

The same contract is surfaced in three places so the active worker and any retry loop see it consistently:

- EAS stage methodology and Prober prompt;
- Task-mode stage charter rendered from `StageSpec`;
- `submit_stage_deliverable` tool schema descriptions for `coverage`, `tested_units`, and `total_units`.

This still does not make self-reported `found` authoritative. DB truth and freshness remain the preferred path, while this slice stops weak explicit cells from passing silently.

## 4. Deferred

- A true EAS semantic gate that reads structured open-port attempt facts from DB and checks `open_ports <= fingerprinted_ports` without relying on coverage denominator fields.
- A first-class active scan attempt table or authoritative active Empty facts. This would require schema/migration approval before implementation.
- Automatic one-layer provider recursion when EAS finds a new apex domain; the `recon_map_assets(domain=...)` capability exists, but wiring from active discoveries is separate.

## 5. Verification

Focused checks:

- `python3 -m json.tool resources/harness/stages/external_attack_surface/spec.json`
- `cd backend && cargo nextest run -p golish-agent-kit external_attack_surface --status-level fail`
- `cd backend && cargo nextest run -p golish-agent-kit coverage_denominator --status-level fail`
- `cd backend && cargo nextest run -p golish-sub-agents prober --status-level fail`

Full `init.sh` / `just precommit` is intentionally not part of this slice because the user requested a direct targeted workflow.

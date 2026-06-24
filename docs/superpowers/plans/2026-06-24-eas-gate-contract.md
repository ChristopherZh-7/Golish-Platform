# EAS gate contract tightening plan

Design: `docs/design/2026-06-24-eas-gate-contract.md`.

Goal: make `external_attack_surface` gate delivery stricter without schema changes: coverage cells need evidence, `blocked` / `not_applicable` need notes, and open-port fingerprinting uses the existing coverage denominator.

## Tasks

- [x] Update `resources/harness/stages/external_attack_surface/spec.json`
  - add coverage evidence rules for `found` and `checked_empty`;
  - set `coverage_complete.require_note_for_other=true`;
  - add `coverage_denominator` with full coverage default.
- [x] Update `resources/harness/stages/external_attack_surface/methodology.md`
  - document asset-type semantics;
  - document denominator fields for liveness, port scan, and service fingerprint.
- [x] Update `build_prober_prompt`
  - require `list_attack_surface_seeds`;
  - make IP/domain/URL/CIDR handling explicit;
  - forbid treating HTTP liveness alone as PORT/SERVICE proof;
  - require denominator fields for explicit coverage cells.
- [x] Update Task-mode stage charter
  - render an EAS-specific asset/port coverage contract;
  - explain URL liveness vs host-level PORT/SERVICE;
  - define SERVICE-FINGERPRINT denominator as fingerprinted open ports over discovered open ports.
- [x] Update `submit_stage_deliverable` schema descriptions
  - include an EAS-specific denominator example;
  - mention open-port fingerprinting in `tested_units` / `total_units` field descriptions.
- [x] Update tests
  - EAS spec gate rule count/shape;
  - EAS happy path still passes;
  - EAS denominator partial blocks;
  - prober prompt, stage charter, and submit schema contain the new contract language.
- [x] Update module cards.
- [x] Update project progress records.

## Non-goals

- No database migration.
- No `authoritative_found` for EAS.
- No automatic new-apex recursion wiring.
- No full `init.sh` / `just precommit` unless the user asks for it.

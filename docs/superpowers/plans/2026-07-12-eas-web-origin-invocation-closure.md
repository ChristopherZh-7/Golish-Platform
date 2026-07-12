# EAS Web-Origin Invocation Closure Implementation Plan

## Goal

Prevent EAS WEB-FINGERPRINT retries from guessing the wrong URL scheme while
preserving the exact-origin authorization boundary.

## Tasks

1. Add RED resolver tests for the narrow unique authorized HTTP-to-HTTPS policy,
   including host/port/owner/scope denial cases.
2. Add RED EAS wrapper tests proving effective HTTPS targets—not caller HTTP
   typos—feed WhatWeb, authorization, evidence identity, and batch ordering.
3. Implement wrapper-only pre-resolution and strict second authorization.
4. Add RED worklist tests for `details.missing_origins` and
   `recommended_args.target_urls`, then preserve those fields.
5. Add RED repair-mode and Prober prompt tests for object target entries and
   exact-origin copying, then update the implementation and methodology.
6. Run focused tests for `golish-pentest-app`, `golish-agent-app`,
   `golish-agent-kit`, and `golish-sub-agents`, followed by scoped Clippy, fmt,
   JSON, and diff checks.
7. Record verification in `agent-progress.md` and the current in-progress
   feature. Keep the feature in progress until a freshly compiled live EAS run
   confirms durable WEB evidence/outcomes.

## Non-goals

- Do not loosen generic browser/Enumeration exact-origin resolution.
- Do not infer HTTPS from port 443.
- Do not change DB schema, migrations, generated IPC types, or expose raw
  WhatWeb/pentest-run arguments.
- Do not make invalid mixed batches partially execute.

## Focused verification status

- TDD RED captured for missing coverage arguments, dropped worklist details,
  rejected object-form repair inputs, missing Prompt constraints, and absent
  wrapper reconciliation implementation.
- GREEN: pentest-app reconciliation 5/5 and EAS capability suite 39/39;
  agent-app stage coverage 87/87; agent-kit stage worklist 5/5; sub-agent
  Prompt/object/string focused regressions all passed.
- Independent four-crate combined run
  `356772e3-9da5-4ca7-b718-da68ca24f315`: 9/9 passed.
- Fresh compiled live EAS acceptance remains pending because it would launch
  an authorized external scan and requires a restarted backend.

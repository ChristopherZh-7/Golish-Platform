# Runtime Supervisor Implementation Plan

Status: in progress
Design: `../../design/2026-06-25-runtime-supervisor.md`

## Goal

Convert the old Mentor-style runtime monitor into a stage-aware RuntimeSupervisor inspired by PentAGI execution monitoring, but bounded by Golish stage specs and repair directives.

## Tasks

1. Add `runtime_supervisor` DTOs and sanitizer in `golish-agent-kit`.
2. Add RuntimeSupervisor prompts that request strict JSON.
3. Add one-shot runtime supervisor LLM helper in `golish-agent-runtime`.
4. Replace main-agent telemetry-only Mentor path with RuntimeSupervisor directive generation and trace emission.
5. Replace sub-agent observer Mentor telemetry with RuntimeSupervisor directive generation and trace emission.
6. Add `HarnessTraceKind::RuntimeSupervisorDecision` and update op-trace, transcript summarizer, and `run_tree.py`.
7. Update module cards and progress metadata.
8. Run scoped Rust build for affected crates.

## Non-Goals

- Do not change deterministic gate PASS/BLOCK logic.
- Do not rewrite StageRefiner.
- Do not run `just precommit` in this user-requested slice.

## Verification

- `cd backend && cargo fmt -p golish-core -p golish-events -p golish-agent-kit -p golish-agent-runtime`
- `cd backend && cargo check -p golish-core -p golish-events -p golish-agent-kit -p golish-agent-runtime`

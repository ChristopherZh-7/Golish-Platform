# Bounded recovery-action model projection

## Status

P0 implementation in progress.

## Problem

An Enumeration gate produced 1,176 structured `coverage_gap_actions`. The
structured vectors are required for deterministic tool authorization, but the
same vectors were also rendered verbatim into `RepairDirective` and
`SubmitRepairMode` prompts. A resumed 69-message worker therefore sent more
than 1.2 million message tokens, and the next retry grew by another 121,810
tokens because the directive text expanded the same action list again.

Message count is not a context-safety boundary. Recovery projections need an
independent deterministic byte and item budget while the internal authorization
data remains lossless.

## Invariants

1. `RepairDirective.actions` and `SubmitRepairMode.coverage_gap_actions` retain
   every action. Tool guards always authorize against the full vectors.
2. Model-visible recovery text exposes the full action count, a stable hash of
   the full ordered vector, and at most the first 20 deterministic samples.
3. Omitted actions are fetched from DB truth with bounded
   `stage_worklist_next` pages; prompt text never pretends that the sample is the
   authorization source.
4. Every recovery instruction is capped at 32 KiB of valid UTF-8, including
   pathological long reason/hint fields.
5. A blocked-tool payload contains only a bounded action projection
   (`total/hash/sample/omitted/next_page_tool`), never the full vector.
6. A StageRefiner-generated `directive_message` already contains the bounded
   projection. `SubmitRepairMode::model_instruction` detects that projection
   and does not append the action list again.
7. Projection order and hash are byte-stable across repeated calls with the
   same ordered structured input.

## Projection contract

The common model-facing header is:

```text
Recovery actions: total=<N> stable_hash=<hash> sample_count=<M>.
Only the bounded sample is shown here. The full action set remains enforced
internally; call stage_worklist_next for the next DB-backed page.
```

Samples preserve the original order. They are diagnostic routing hints only;
the existing full-vector guards continue to reject any target/technique pair
that is not present in the complete action set.

The hash is an informational deterministic fingerprint, not an authorization
token. StageRefiner keeps its existing SHA-256-derived gap hash. The lower
`golish-sub-agents` layer uses an explicitly specified stable FNV-1a fingerprint
over serialized structured actions to avoid introducing an upward dependency.

## Compatibility

- Persistent/checkpoint DTOs stay structurally unchanged.
- Raw gate results and transcripts remain complete.
- Tool allow/deny behavior remains based on the full structured vectors.
- The model-visible `coverage_gap_actions` field in a blocked-tool result changes
  from a full array to a bounded projection object. It is an executor response,
  not a durable DTO.

## Verification

- Red-to-green tests use exactly 1,176 actions.
- Assertions cover total/hash/sample visibility, absence of action 21 and later
  identities, 32 KiB text bounds, 64 KiB blocked-payload bounds, full internal
  vector retention, and repeated-call byte stability.
- Scoped crate tests, formatting, and clippy must pass without touching the DB
  or a live run.

# R5 — `rig-core` Provider Fork Maintenance Burden

> Status: **Risk identified, mitigation = staged upstream PRs.**
> Last updated: 2026-05-02.

## Current state

The workspace ships **four in-tree forks** of `rig-core` provider crates:

| Crate | Why forked |
|---|---|
| `rig-anthropic-vertex` | Claude on Google Vertex AI auth flow not in upstream |
| `rig-openai-responses` | OpenAI `/v1/responses` reasoning-model API not in upstream `rig-openai` |
| `rig-gemini-vertex` | Gemini on Vertex AI; companion to `rig-vertexai` |
| `rig-zai-sdk` | Z.AI GLM provider — not in upstream |

Two extra crates `golish-json-repair` exist because LLM responses
malform JSON often enough to need a dedicated repair layer
(typically pulled in by the OpenAI Responses fork).

## Cost over time

- Every `rig-core` upstream release: 4 forks need diff + rebase.
  Last 12 months: 6 releases → ~30h of maintenance time.
- New rig features (e.g. agent middleware, streaming abstractions)
  arrive in upstream first; forks lag by 1-2 minor versions.
- New contributors confused by "wait, are we using rig 0.36 or our
  fork's 0.36?" — onboarding tax.

## Mitigation tracks

### Track 1 — Push fixes upstream
For each fork, classify changes:

| Fork | Upstreamable? | Action |
|---|---|---|
| `rig-anthropic-vertex` | **Yes** — generic Vertex AI auth helper applies to all providers | Open PR adding a `vertex_auth` shared module to `rig-core`, then re-base our Claude fork on it |
| `rig-openai-responses` | **Yes** — `/v1/responses` is now stable, upstream should support | Open PR adding a `ResponseModel` type to `rig-openai` |
| `rig-gemini-vertex` | **Maybe** — depends if upstream `rig-vertexai` 0.2.4 already covers Gemini | Audit `rig-vertexai` source first |
| `rig-zai-sdk` | **No** (provider-specific) | Keep fork; consider donating as `rig-zai` standalone crate |

Estimated upstream PR work: 3-5 days each.

### Track 2 — Reduce fork divergence
For everything that **must** stay forked:
1. Pin upstream version exactly (no `^0.36`, just `=0.36.0`).
2. Add `forks/README.md` listing every diff vs upstream + commit
   hash of the upstream version we're tracking.
3. Add `cargo-deny` rule to fail CI if a fork drifts from the
   pinned upstream version without an explicit owner sign-off.

### Track 3 — Switch to plugin trait
The cleanest long-term answer: factor `rig-core::CompletionModel`
into a trait + dynamic dispatch so providers are pluggable
without touching `rig-core`. This is a 2-week refactor of `rig-core`
itself; would benefit the whole rig ecosystem.

## Recommendation

1. **Immediate (this quarter)**: Track 2 — pin + diff log. ~2h work.
2. **Next quarter**: open the 2 most upstream-able PRs (Anthropic
   Vertex auth + OpenAI Responses). 5-10 days.
3. **6 months out**: re-evaluate. If upstream merges land, retire
   the corresponding forks. If not, consider Track 3.

## References

- `backend/crates/rig-anthropic-vertex/`, `rig-openai-responses/`,
  `rig-gemini-vertex/`, `rig-zai-sdk/`
- `backend/Cargo.toml:218` — `rig-core = "^0.36.0"`
- ADR-0002 (rig-core vs LangChain-rs) — `docs/adr/0002-rig-core-vs-langchain-rs.md`
- Upstream rig: <https://github.com/0xPlaygrounds/rig>

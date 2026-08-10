# Candidate Technique Method Contracts

> Status: implementation design for the 2026-08-02 Candidate hardening slice.
> This document extends `2026-07-29-tool-truth-hypothesis-verification-loop.md`;
> it does not replace Plan C campaign authority.

## Decision

Candidate hypothesis generation must stop treating an arbitrary non-empty
`predicate_schema` as sufficient hypothesis authority. Every proposal is bound
to one server-owned, versioned Technique Method Contract. The contract tells the
analysis team when a technique may be considered, which prerequisites must be
assessed, which controls Verification must preserve, and which oracle profile
can later promote observations to a Finding.

The registry is authored inside Golish from primary OWASP WSTG/CWE concepts.
CyberStrike's public skills inspired the card shape (version, prerequisites,
chains, test procedure and Finding criteria), but their prose, payloads, scripts
and code are not copied. The runtime does not load CyberStrike or use its model.

## Authority split

1. Tool Truth and Application Context remain fact authority.
2. A Technique Method Contract is methodology authority, not target evidence.
3. Candidate binds exact evidence refs to a card and assesses every card
   prerequisite as `satisfied`, `missing`, or `unknown`.
4. Any missing/unknown prerequisite forces `needs_enrichment`; it creates a
   durable hypothesis/gap, never a runnable exploit assertion.
5. Verification compiles the pinned card digest into its contract. A changed or
   unknown digest fails closed.
6. Only a typed oracle over landed evidence can create a Finding.

## One product stage, two internal authorities

The roadmap and user workflow should present Candidate plus Verification as one
`Hypothesis Validation Loop`, not two disconnected hand-offs. The loop reads all
prior asset/application facts, proposes a bounded hypothesis set, validates the
runnable subset, lands FactDelta/evidence, re-analyzes the new facts and repeats
until every hypothesis is verified, refuted, held for enrichment/capability, or
the bounded fixed point is sealed.

This product-stage merge does not merge privileges. Hypothesis analysts remain
read-only; target-touching work still crosses Prepared Action, scope, budget and
authorization gates; only typed oracles can adjudicate evidence. A single
all-powerful model must never propose, execute and declare its own Finding.

## Contract

`CandidateTechniqueMethodCardV1` contains:

- stable technique id, version and deterministic digest;
- attack-class and predicate schema/version binding;
- versioned framework references and CWE identifiers;
- closed applicability signal identifiers;
- exact prerequisite identifiers;
- required experimental phases: baseline, attack, negative control,
  reproduction, impact proof and cleanup;
- oracle profile id/version;
- context-change triggers that require re-evaluation.

`CandidateTechniqueBindingV1` is model-authored only by selecting bytes already
provided by the host. It contains the exact card identity/digest, one or more
matched signal ids, and exactly one assessment for every prerequisite. Each
assessment cites a delivered frozen chunk: satisfied uses support/application
context; missing/unknown uses a gap reference.

## Deterministic admission rules

- Unknown technique, predicate drift, version drift or digest drift is rejected.
- Matched signals must be a non-empty subset of the selected card.
- Prerequisite ids must be an exact set with no duplicates.
- A satisfied prerequisite requires an authoritative support/context reference.
- A missing or unknown prerequisite requires a gap reference.
- `ready_for_strategy` is legal only when every prerequisite is satisfied.
- A knowledge signal alone can never satisfy applicability.
- The method-card digest is included in revision ingredients and the compiled
  Verification rule/prerequisite authority.
- Candidate still cannot execute tools or write Findings.

## Initial catalog

The first catalog covers the current Golish attack classes and the high-value
web/API techniques represented by the reviewed methodology: authentication,
IDOR/authorization, business logic, race conditions, configuration, data
exposure, SQL/input/command injection, SSRF, SSTI, request smuggling, CORS, JWT,
GraphQL, host-header handling, open redirect, prototype pollution, rate-limit
bypass, subdomain takeover, WebSocket, XXE, cache poisoning, availability and
known vulnerable components.

Catalog breadth does not claim adapter breadth. A method card can yield a valid
planning hypothesis while capability assessment remains `adapter_missing`.

## Persistence and compatibility

This slice requires no migration and no generated IPC change. Existing JSONB
proposal and compiler-recipe envelopes carry the additive typed binding. Fresh
Plan B Candidate analysis requires it; legacy Candidate/Attempt rows remain
read-only compatibility data and are not upgraded in place.

## Verification scope

- pure registry digest, lookup and exact-set tests in `golish-core`;
- analyst-input and prompt tests in `golish-agent-app`/`golish-sub-agents`;
- repository tests for unknown/drifted cards, incomplete prerequisite sets,
  readiness escalation and reference-role mismatch;
- compiler tests proving the card digest reaches Verification authority;
- affected-crate Clippy with warnings denied.

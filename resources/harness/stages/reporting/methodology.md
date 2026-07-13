# Reporting methodology

Reporting is a deterministic read-model closeout stage. It does not ask the
model to invent, summarize, or select engagement facts.

## Server-owned workflow

1. Freeze the operation's organization scope and read the complete canonical
   source set from PostgreSQL.
2. Build or reuse the current report revision from those sources only.
3. Require every narrative claim to resolve to same-operation evidence and
   retain every consumed and unconsumed canonical source in the source
   manifest.
4. Re-read current source truth and validate the exact source-set hash,
   claim/citation integrity, validation attestation, and Cleanup closeout.
5. Expose the validated read model for review. Artifact rendering and final
   publication remain separate, explicit operator actions.

## Agent boundary

The server prepares the canonical validated revision before the reporting agent
runs. The agent may only acknowledge that the stage read model is ready and
submit the minimal `StageDeliverable` required by the harness. Include exactly
one non-authoritative acknowledgement claim with `kind=report_read_model_ready`,
`subject=canonical_report`, a short readiness summary, and no evidence ids or
technique. This only satisfies the structural non-vacuous check; the Reporting
Gate ignores it and reads DB truth. The agent must not:

- scan targets or create new pentest facts;
- use RAG, knowledge-graph, wiki, or free-form prose as report authority;
- fabricate evidence identifiers, citations, findings, or cleanup state;
- render or publish artifacts; or
- claim final publication on the operator's behalf.

If the deterministic reporting gate blocks, rebuild from current DB truth or
repair the upstream canonical source/evidence relation named by the gate. Never
work around a block by changing model prose.

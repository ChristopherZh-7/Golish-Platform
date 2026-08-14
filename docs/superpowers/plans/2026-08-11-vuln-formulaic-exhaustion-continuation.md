# Vuln formulaic exhaustion continuation implementation plan

1. Add DB-free parsers/predicates for the anonymous-access batch and the
   server-authored narrowed recovery request. Cover exact acceptance and
   malformed/foreign/non-timeout rejection.
2. Extend `seal_exhausted_vuln_residual_outcomes` without changing the existing
   Nuclei branch. Bind anonymous evidence to its exact child request, Worker and
   blocked output, then append source-specific terminalization evidence.
3. Add an additive migration and repository fallback for one exact exhausted
   open formulaic Controller whose Vuln denominator is now fully terminal.
   Expose it through the existing leader-claim seam.
4. Run the pre-claim residual reconciliation from the server worklist runtime
   using the seeded terminal leader witness, then continue through the normal
   prepare-final/final-submission path.
5. Run focused DB/runtime tests, scoped Clippy/rustfmt/diff checks, then rebuild
   the dev binary and verify the retained operation advances without a repeated
   anonymous-access network call.

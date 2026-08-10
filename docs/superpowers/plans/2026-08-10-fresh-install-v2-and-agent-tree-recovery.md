# Fresh-install V2 and Agent tree recovery plan

1. Add a pristine-only forward migration that selects the accepted rollout contracts and writes an
   immutable bootstrap receipt. Prove both the SQL guard shape and a real isolated migration run.
2. Restore the final Stage Agent workspace components from the recovered dirty main worktree,
   without replacing the newer Investigation, Candidate, or Reporting routes.
3. Route ChatPanel `stage_run` and SubAgent cards to one Tool Detail surface, preserving exact
   stage/Worker request identity and fail-closed historical behavior.
4. Restore message-scoped Stage Plan cards, immutable stage-to-assistant-message anchors,
   refresh-safe anchor persistence, and the compact always-visible workflow status strip.
5. Make the typed Company Controller detail replace the generic Tool envelope and own a fixed
   height with an internally scrolling Agent transcript.
6. Adapt current-window Target Intel semantic observations into the existing explicit pre-EAS scope
   review, keeping candidate evidence separate from the resulting customer authorization and
   rejecting foreign, stale, misclassified, duplicate, or drifted candidates.
7. Run focused workspace/navigation/detail/Plan tests, semantic-scope bridge tests, TypeScript no-emit,
   scoped Biome, scoped Rust Clippy, rustfmt, and
   diff checks.
8. Start a fresh desktop database, confirm the accepted frozen contracts, and inspect the fresh
   Target Intel Stage Team rows for a real Controller/Worker chain.
9. Update module cards, feature evidence, and progress; create one bug-fix commit and push the
   current recovery branch only after all scoped checks pass.

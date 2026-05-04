/**
 * Timeline utilities for unified timeline rendering.
 *
 * Most consumers import directly from the sub-modules
 * (e.g. `@/lib/timeline/selectors`, `@/lib/timeline/blockHeightEstimation`,
 * `@/lib/timeline/streamingBlockFinalization`). This barrel keeps a small
 * surface for the sub-agent extraction helpers used by the AgentMessage
 * renderer; previous re-exports of selectors / height estimation /
 * streaming finalization were unused (M2.3 cleanup).
 */

export { extractSubAgentBlocks, type RenderBlock } from "./subAgentExtraction";

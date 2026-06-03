/**
 * Severity for an assistant "error" message in the chat.
 *
 * `error` (red) = a real failure the user must care about (crash, auth, network).
 * `warning` (amber) = a soft, often recoverable condition surfaced through the
 * same error channel — e.g. the Task planner replying in prose instead of a plan
 * because the user said "hi" rather than describing a task. These should not look
 * like hard failures.
 */
export type MessageSeverity = "error" | "warning";

/**
 * Stable, backend-generated English phrases that mark a soft condition. Matched
 * case-insensitively against the surfaced error text so the same classification
 * holds whether it arrives as an `error` event or an invoke rejection (which
 * wraps the same message with an `[API trace=…]` prefix).
 *
 * Keep these aligned with the backend strings (e.g.
 * `describe_plan_parse_failure` in golish-agent-bridge).
 */
const WARNING_SIGNALS = [
  "declined to produce a plan",
  "returned a message instead of a plan",
  "refused the request or asked a question",
  // The planner sometimes wraps a clarification in JSON ({"message": "…"}),
  // which fails plan parsing as "Failed to parse <phase> JSON". Still a soft
  // "tell me your task" prompt, not a hard failure.
  "failed to parse task planner json",
] as const;

/** Classify a surfaced error string as a hard `error` or a soft `warning`. */
export function classifyErrorSeverity(message: string): MessageSeverity {
  const lower = message.toLowerCase();
  if (WARNING_SIGNALS.some((signal) => lower.includes(signal))) return "warning";
  // A planner reply that is valid JSON but missing the required `subtasks`
  // field is the model answering conversationally (e.g. `{"message": "…"}`)
  // instead of emitting a plan — treat as a soft warning, not a red error.
  if (lower.includes("missing field") && lower.includes("subtasks")) return "warning";
  return "error";
}

/**
 * TypeScript equivalent of Rust's `serde_json::Value`.
 *
 * Used as the type of arbitrary JSON values returned via Tauri command
 * results (tool / invoke responses) where the payload schema is not
 * statically known. Originally added as a helper for ts-rs generated
 * types; after the M2.5 codegen removal it is kept because several
 * frontend modules (lib/ai/types, services/ai-events/*) still need
 * a `JsonValue` type for untyped JSON payloads.
 */
export type JsonValue =
  | string
  | number
  | boolean
  | null
  | JsonValue[]
  | { [key: string]: JsonValue };

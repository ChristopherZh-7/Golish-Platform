/**
 * TypeScript equivalent of Rust's serde_json::Value.
 * Used by ts-rs generated types.
 */
export type JsonValue =
  | string
  | number
  | boolean
  | null
  | JsonValue[]
  | { [key: string]: JsonValue };

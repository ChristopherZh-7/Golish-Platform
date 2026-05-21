/**
 * Polymorphic field renderer.
 *
 * Picks the right concrete component for a schema-declared
 * [`Field`] based on its `type`. Centralising the switch here keeps
 * `<IntegrationGroup>` free of field-type knowledge.
 */

import type { Field, FieldValue } from "@/lib/api/integrations";
import { BooleanField } from "./BooleanField";
import { ProxyInput } from "./ProxyInput";
import { SecretInput } from "./SecretInput";
import { SecretTextarea } from "./SecretTextarea";
import { SelectField } from "./SelectField";
import { TextInput } from "./TextInput";

interface FieldRendererProps {
  field: Field;
  value: string;
  onChange: (next: string) => void;
  disabled?: boolean;
  /** When the backend reports the field is already configured (even
   * for secrets where the cleartext isn't surfaced), this lets the
   * input display a "configured, hidden" placeholder. */
  serverValue?: FieldValue | null;
  id?: string;
}

export function FieldRenderer({
  field,
  value,
  onChange,
  disabled,
  serverValue,
  id,
}: FieldRendererProps) {
  const placeholderForExisting =
    serverValue?.has_value && (field.type === "secret_text" || field.type === "secret_textarea")
      ? serverValue.display_hint || "•••• (configured)"
      : undefined;
  const hasExistingSecret =
    Boolean(serverValue?.has_value) &&
    (field.type === "secret_text" || field.type === "secret_textarea");

  switch (field.type) {
    case "secret_text":
      return (
        <SecretInput
          id={id}
          value={value}
          onChange={onChange}
          placeholder={field.placeholder}
          disabled={disabled}
          placeholderForExistingSecret={placeholderForExisting}
          hasExistingSecret={hasExistingSecret}
        />
      );
    case "secret_textarea":
      return (
        <SecretTextarea
          id={id}
          value={value}
          onChange={onChange}
          placeholder={field.placeholder}
          disabled={disabled}
          rows={field.rows}
          placeholderForExistingSecret={placeholderForExisting}
          hasExistingSecret={hasExistingSecret}
        />
      );
    case "url":
      return (
        <TextInput
          id={id}
          value={value}
          onChange={onChange}
          type="url"
          inputMode="url"
          placeholder={field.placeholder ?? "https://"}
          pattern={field.pattern}
          disabled={disabled}
        />
      );
    case "port":
      return (
        <TextInput
          id={id}
          value={value}
          onChange={onChange}
          type="number"
          inputMode="numeric"
          placeholder={field.placeholder}
          min={1}
          max={65535}
          disabled={disabled}
        />
      );
    case "select":
      return (
        <SelectField
          id={id}
          value={value}
          onChange={onChange}
          options={field.options ?? []}
          placeholder={field.placeholder}
          disabled={disabled}
        />
      );
    case "boolean":
      return (
        <BooleanField
          id={id}
          value={value}
          onChange={onChange}
          label={field.placeholder}
          disabled={disabled}
        />
      );
    case "proxy":
      return (
        <ProxyInput
          id={id}
          value={value}
          onChange={onChange}
          placeholder={field.placeholder}
          disabled={disabled}
        />
      );
    default:
      // Covers `"text"` plus any future field type we don't yet have
      // a dedicated renderer for — render as a plain text input.
      return (
        <TextInput
          id={id}
          value={value}
          onChange={onChange}
          placeholder={field.placeholder}
          pattern={field.pattern}
          disabled={disabled}
        />
      );
  }
}

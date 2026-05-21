import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { SecretInput } from "./SecretInput";

describe("SecretInput", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("renders as type='password' by default", () => {
    const onChange = vi.fn();
    render(<SecretInput value="secret-value" onChange={onChange} />);
    const input = screen.getByDisplayValue("secret-value") as HTMLInputElement;
    expect(input.type).toBe("password");
  });

  it("toggles to text on reveal click and back on second click", () => {
    const onChange = vi.fn();
    render(<SecretInput value="secret-value" onChange={onChange} />);
    const input = screen.getByDisplayValue("secret-value") as HTMLInputElement;
    const button = screen.getByRole("button", { name: /toggle visibility/i });

    expect(input.type).toBe("password");

    fireEvent.click(button);
    expect(input.type).toBe("text");

    fireEvent.click(button);
    expect(input.type).toBe("password");
  });

  it("re-masks automatically after 30 seconds of being revealed", () => {
    const onChange = vi.fn();
    render(<SecretInput value="secret-value" onChange={onChange} />);
    const input = screen.getByDisplayValue("secret-value") as HTMLInputElement;
    const button = screen.getByRole("button", { name: /toggle visibility/i });

    fireEvent.click(button);
    expect(input.type).toBe("text");

    // Just under 30s: still visible
    act(() => {
      vi.advanceTimersByTime(29_000);
    });
    expect(input.type).toBe("text");

    // Past 30s: auto-masked
    act(() => {
      vi.advanceTimersByTime(2_000);
    });
    expect(input.type).toBe("password");
  });

  it("propagates value changes through onChange", () => {
    const onChange = vi.fn();
    render(<SecretInput value="" onChange={onChange} placeholder="enter secret" />);
    const input = screen.getByPlaceholderText("enter secret") as HTMLInputElement;
    fireEvent.change(input, { target: { value: "new-secret" } });
    expect(onChange).toHaveBeenCalledWith("new-secret");
  });

  it("shows the 'existing secret' placeholder when value is empty and the prop is set", () => {
    render(
      <SecretInput
        value=""
        onChange={vi.fn()}
        placeholderForExistingSecret="•••• (configured)"
        placeholder="enter secret"
      />
    );
    // The "configured" hint wins over the regular placeholder when
    // the input is empty AND the server reported a configured value.
    const input = screen.getByPlaceholderText("•••• (configured)");
    expect(input).toBeInTheDocument();
  });
});

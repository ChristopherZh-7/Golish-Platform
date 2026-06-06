import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { normalizeScopeRows, ScopeReviewTable } from "./ScopeReviewTable";

describe("normalizeScopeRows", () => {
  it("returns one blank row for non-array / empty input", () => {
    expect(normalizeScopeRows("scope_review", null)).toHaveLength(1);
    expect(normalizeScopeRows("scope_review", [])).toHaveLength(1);
    // Select columns default to their first option.
    expect(normalizeScopeRows("scope_review", null)[0]).toEqual({
      value: "",
      type: "domain",
      scope: "in",
    });
  });

  it("maps proposed items into columns and joins array cells", () => {
    const rows = normalizeScopeRows("unit_review", [
      { name: "Acme", aliases: ["ACME Inc", "Acme Co"], domains: ["acme.com"] },
    ]);
    expect(rows[0].name).toBe("Acme");
    expect(rows[0].aliases).toBe("ACME Inc, Acme Co");
    expect(rows[0].domains).toBe("acme.com");
  });
});

describe("ScopeReviewTable", () => {
  it("confirms the edited rows as a JSON-serialisable array", async () => {
    const user = userEvent.setup();
    const onConfirm = vi.fn();
    render(
      <ScopeReviewTable
        kind="scope_review"
        initial={[{ value: "example.com", type: "domain", scope: "in" }]}
        onConfirm={onConfirm}
        onSkip={vi.fn()}
      />
    );

    // Edit the first target value.
    const valueInput = screen.getByLabelText("Target for row 1");
    await user.clear(valueInput);
    await user.type(valueInput, "api.example.com");

    // Add a second row and fill it.
    await user.click(screen.getByRole("button", { name: "Add row" }));
    await user.type(screen.getByLabelText("Target for row 2"), "10.0.0.0/24");

    await user.click(screen.getByRole("button", { name: "Confirm" }));

    expect(onConfirm).toHaveBeenCalledTimes(1);
    const rows = onConfirm.mock.calls[0][0];
    expect(rows).toHaveLength(2);
    expect(rows[0].value).toBe("api.example.com");
    expect(rows[1].value).toBe("10.0.0.0/24");
    // The caller submits JSON.stringify(rows); confirm it round-trips.
    expect(() => JSON.parse(JSON.stringify(rows))).not.toThrow();
  });

  it("removes a row and drops blank rows on confirm", async () => {
    const user = userEvent.setup();
    const onConfirm = vi.fn();
    render(
      <ScopeReviewTable
        kind="scope_review"
        initial={[
          { value: "keep.com", type: "domain", scope: "in" },
          { value: "remove.com", type: "domain", scope: "out" },
        ]}
        onConfirm={onConfirm}
        onSkip={vi.fn()}
      />
    );

    await user.click(screen.getByRole("button", { name: "Remove row 2" }));
    await user.click(screen.getByRole("button", { name: "Confirm" }));

    const rows = onConfirm.mock.calls[0][0];
    expect(rows).toHaveLength(1);
    expect(rows[0].value).toBe("keep.com");
  });

  it("invokes onSkip when skipped", async () => {
    const user = userEvent.setup();
    const onSkip = vi.fn();
    render(<ScopeReviewTable kind="unit_review" initial={[]} onConfirm={vi.fn()} onSkip={onSkip} />);
    await user.click(screen.getByRole("button", { name: "Skip" }));
    expect(onSkip).toHaveBeenCalledTimes(1);
  });
});

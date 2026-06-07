import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import {
  detectTargetType,
  normalizeScopeRows,
  parseBulkRows,
  ScopeReviewTable,
} from "./ScopeReviewTable";

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

describe("detectTargetType", () => {
  it("classifies common target shapes", () => {
    expect(detectTargetType("example.com")).toBe("domain");
    expect(detectTargetType("*.example.com")).toBe("wildcard");
    expect(detectTargetType("10.0.0.5")).toBe("ip");
    expect(detectTargetType("10.0.0.0/24")).toBe("cidr");
    expect(detectTargetType("https://app.example.com")).toBe("url");
    expect(detectTargetType("app.example.com/login")).toBe("url");
    expect(detectTargetType("fe80::1")).toBe("ip");
    expect(detectTargetType("2001:db8::/32")).toBe("cidr");
  });
});

describe("parseBulkRows", () => {
  it("parses a newline + comma list into typed scope rows", () => {
    const rows = parseBulkRows(
      "scope_review",
      "example.com, *.example.com\n10.0.0.0/24\nhttps://app.example.com"
    );
    expect(rows).toEqual([
      { value: "example.com", type: "domain", scope: "in" },
      { value: "*.example.com", type: "wildcard", scope: "in" },
      { value: "10.0.0.0/24", type: "cidr", scope: "in" },
      { value: "https://app.example.com", type: "url", scope: "in" },
    ]);
  });

  it("strips markdown bullets and table noise, keeping only target-like tokens", () => {
    const rows = parseBulkRows(
      "scope_review",
      "- example.com\n* api.example.com\n| 1 | pingan.com | domain | in |"
    );
    expect(rows.map((r) => r.value)).toEqual(["example.com", "api.example.com", "pingan.com"]);
  });

  it("treats each line as one organisation for unit_review", () => {
    const rows = parseBulkRows("unit_review", "Acme Corp\nAcme Subsidiary\n");
    expect(rows).toEqual([
      { name: "Acme Corp", aliases: "", domains: "" },
      { name: "Acme Subsidiary", aliases: "", domains: "" },
    ]);
  });

  it("de-duplicates repeated targets case-insensitively", () => {
    const rows = parseBulkRows("scope_review", "a.com\n10.0.0.1\nA.com\na.com");
    expect(rows.map((r) => r.value)).toEqual(["a.com", "10.0.0.1"]);
  });
});

describe("ScopeReviewTable", () => {
  it("shows the free-text editor expanded by default with no grid", () => {
    render(
      <ScopeReviewTable kind="scope_review" initial={[]} onConfirm={vi.fn()} onSkip={vi.fn()} />
    );
    // Textarea is visible immediately — no toggle click needed.
    expect(screen.getByLabelText("Bulk targets")).toBeInTheDocument();
    // The row-by-row grid is gone.
    expect(screen.queryByRole("button", { name: "Add row" })).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Target for row 1")).not.toBeInTheDocument();
  });

  it("confirms typed targets as parsed, de-duplicated, JSON-serialisable rows", async () => {
    const user = userEvent.setup();
    const onConfirm = vi.fn();
    render(
      <ScopeReviewTable kind="scope_review" initial={[]} onConfirm={onConfirm} onSkip={vi.fn()} />
    );

    await user.type(screen.getByLabelText("Bulk targets"), "a.com{Enter}10.0.0.1{Enter}a.com");
    await user.click(screen.getByRole("button", { name: "Confirm" }));

    expect(onConfirm).toHaveBeenCalledTimes(1);
    const rows = onConfirm.mock.calls[0][0];
    expect(rows).toEqual([
      { value: "a.com", type: "domain", scope: "in" },
      { value: "10.0.0.1", type: "ip", scope: "in" },
    ]);
    expect(() => JSON.parse(JSON.stringify(rows))).not.toThrow();
  });

  it("seeds the textarea from AI-proposed initial targets", () => {
    render(
      <ScopeReviewTable
        kind="scope_review"
        initial={[
          { value: "example.com", type: "domain", scope: "in" },
          { value: "*.example.com", type: "wildcard", scope: "in" },
        ]}
        onConfirm={vi.fn()}
        onSkip={vi.fn()}
      />
    );
    expect(screen.getByLabelText("Bulk targets")).toHaveValue("example.com\n*.example.com");
  });

  it("invokes onSkip when skipped", async () => {
    const user = userEvent.setup();
    const onSkip = vi.fn();
    render(<ScopeReviewTable kind="unit_review" initial={[]} onConfirm={vi.fn()} onSkip={onSkip} />);
    await user.click(screen.getByRole("button", { name: "Skip" }));
    expect(onSkip).toHaveBeenCalledTimes(1);
  });
});

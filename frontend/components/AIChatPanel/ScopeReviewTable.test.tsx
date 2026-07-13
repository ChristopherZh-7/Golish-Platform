import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createRef } from "react";
import { describe, expect, it, vi } from "vitest";
import type { OrganizationCandidate } from "@/lib/api/organizations";
import {
  candidatesToUnitRows,
  detectTargetType,
  normalizeScopeRows,
  parseBulkRows,
  parseOwnershipPercent,
  type ScopeReviewHandle,
  ScopeReviewTable,
} from "./ScopeReviewTable";

function candidate(
  id: string,
  value: string,
  evidence: unknown = {},
  extra: Partial<OrganizationCandidate> = {}
): OrganizationCandidate {
  return {
    id,
    kind: "organization",
    label: value,
    value,
    source: "test",
    confidence: 1,
    status: "needs_review",
    evidence,
    createdAt: 1,
    ...extra,
  };
}

describe("candidatesToUnitRows", () => {
  it("maps org candidates to stable review rows and drops empties", () => {
    const rows = candidatesToUnitRows([
      candidate("cand-bank", "平安银行股份有限公司", { raw: { scale: "58%" } }, {
        organizationId: "org-bank",
        ownershipPercent: "58%",
      }),
      candidate("cand-unknown", "无比例公司"),
      candidate("cand-empty", "   "),
    ]);
    expect(rows).toHaveLength(2);
    expect(rows[0]).toEqual({
      reviewRowId: "candidate:cand-bank",
      candidateId: "cand-bank",
      organizationId: "org-bank",
      name: "平安银行股份有限公司",
      aliases: [],
      domains: [],
      ownershipPercent: "58%",
      included: true,
    });
    expect(rows[1]).toEqual(
      expect.objectContaining({
        reviewRowId: "candidate:cand-unknown",
        candidateId: "cand-unknown",
        organizationId: null,
        name: "无比例公司",
        ownershipPercent: null,
      })
    );
  });

  it("drops below-threshold and unknown-ownership candidates when a threshold is given", () => {
    const rows = candidatesToUnitRows(
      [
        candidate("cand-bank", "平安银行", { raw: { scale: "58%" } }),
        candidate("cand-doctor", "平安好医生", { raw: { scale: "39.41%" } }),
        candidate("cand-unknown", "无比例公司"),
        candidate("cand-securities", "平安证券", { raw: { scale: "100%" } }),
      ],
      51
    );
    // Only the ≥51% ones with a parseable scale survive — no hand-deletion.
    expect(rows.map((r) => r.name)).toEqual(["平安银行", "平安证券"]);
  });
});

describe("parseOwnershipPercent", () => {
  it("parses percent strings (mirrors backend parse_ownership_percent) and rejects junk", () => {
    expect(parseOwnershipPercent("58%")).toBe(58);
    expect(parseOwnershipPercent("99.8809%")).toBeCloseTo(99.8809);
    expect(parseOwnershipPercent("12,345")).toBe(12345);
    expect(parseOwnershipPercent("")).toBeNull();
    expect(parseOwnershipPercent(undefined)).toBeNull();
    expect(parseOwnershipPercent("n/a")).toBeNull();
  });
});

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
  it("preserves immutable candidate and organization ids while editing and toggling inclusion", async () => {
    const user = userEvent.setup();
    const onConfirm = vi.fn();
    render(
      <ScopeReviewTable
        kind="unit_review"
        initial={[
          {
            reviewRowId: "candidate:cand-1",
            candidateId: "cand-1",
            organizationId: "org-1",
            name: "Child A",
            aliases: ["A"],
            domains: ["a.example"],
            ownershipPercent: "51%",
            included: true,
          },
        ]}
        onConfirm={onConfirm}
        onSkip={vi.fn()}
      />
    );

    await user.clear(screen.getByLabelText("Name for unit 1"));
    await user.type(screen.getByLabelText("Name for unit 1"), "Child A Renamed");
    await user.click(screen.getByRole("checkbox", { name: "Include Child A Renamed" }));
    await user.click(screen.getByRole("button", { name: "Confirm" }));

    expect(onConfirm).toHaveBeenCalledWith([
      expect.objectContaining({
        reviewRowId: "candidate:cand-1",
        candidateId: "cand-1",
        organizationId: "org-1",
        name: "Child A Renamed",
        included: false,
      }),
    ]);
  });

  it("assigns a stable review row id to a manually-added unit", async () => {
    const user = userEvent.setup();
    const onConfirm = vi.fn();
    const randomUuid = vi
      .spyOn(crypto, "randomUUID")
      .mockReturnValue("11111111-2222-4333-8444-555555555555");
    render(
      <ScopeReviewTable
        kind="unit_review"
        initial={[]}
        onConfirm={onConfirm}
        onSkip={vi.fn()}
      />
    );

    await user.click(screen.getByRole("button", { name: "Add row" }));
    const names = screen.getAllByLabelText(/Name for unit/);
    await user.type(names[names.length - 1], "Manual Child");
    await user.click(screen.getByRole("button", { name: "Confirm" }));

    expect(onConfirm).toHaveBeenCalledWith(
      expect.arrayContaining([
        expect.objectContaining({
          reviewRowId: "11111111-2222-4333-8444-555555555555",
          candidateId: "",
          organizationId: null,
          name: "Manual Child",
          included: true,
        }),
      ])
    );
    randomUuid.mockRestore();
  });

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

  it("preserves explicit type and out-of-scope state when approving unchanged rows", async () => {
    const user = userEvent.setup();
    const onConfirm = vi.fn();
    render(
      <ScopeReviewTable
        kind="scope_review"
        initial={[{ value: "legacy.example.com", type: "domain", scope: "out" }]}
        onConfirm={onConfirm}
        onSkip={vi.fn()}
      />
    );

    await user.click(screen.getByRole("button", { name: "Confirm" }));
    expect(onConfirm).toHaveBeenCalledWith([
      { value: "legacy.example.com", type: "domain", scope: "out" },
    ]);
  });

  it("invokes onSkip when skipped", async () => {
    const user = userEvent.setup();
    const onSkip = vi.fn();
    render(<ScopeReviewTable kind="unit_review" initial={[]} onConfirm={vi.fn()} onSkip={onSkip} />);
    await user.click(screen.getByRole("button", { name: "Skip" }));
    expect(onSkip).toHaveBeenCalledTimes(1);
  });

  it("exposes an imperative confirm() that submits the latest edited text", async () => {
    const user = userEvent.setup();
    const onConfirm = vi.fn();
    const ref = createRef<ScopeReviewHandle>();
    render(
      <ScopeReviewTable
        ref={ref}
        kind="scope_review"
        initial={[]}
        onConfirm={onConfirm}
        onSkip={vi.fn()}
      />
    );

    // Edit after mount, then confirm via the handle (mirrors the auto-confirm
    // countdown path) — it must use the current text, not the empty seed.
    await user.type(screen.getByLabelText("Bulk targets"), "late.example.com");
    ref.current?.confirm();

    expect(onConfirm).toHaveBeenCalledTimes(1);
    expect(onConfirm).toHaveBeenCalledWith([
      { value: "late.example.com", type: "domain", scope: "in" },
    ]);
  });
});

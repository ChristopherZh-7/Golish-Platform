import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  listOrganizationCandidates,
  type OrganizationCandidate,
} from "@/lib/api/organizations";
import {
  ASK_HUMAN_COUNTDOWN_MS,
  AskHumanInline,
  type AskHumanState,
  parseReviewContext,
  resolveAskHumanInputType,
} from "./AskHumanInline";

vi.mock("@/lib/api/organizations", () => ({
  listOrganizationCandidates: vi.fn(),
}));

function makeRequest(partial: Partial<AskHumanState> = {}): AskHumanState {
  return {
    requestId: "req-1",
    sessionId: "sess-1",
    question: "Which target should I scope?",
    inputType: "choice",
    options: ["Production", "Staging"],
    context: "",
    ...partial,
  };
}

describe("AskHumanInline", () => {
  it("submits the clicked option directly (one-click, Cursor-style)", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    render(<AskHumanInline request={makeRequest()} onSubmit={onSubmit} onSkip={vi.fn()} />);

    // Options carry A/B/… quick-pick badges.
    expect(screen.getByText("A")).toBeInTheDocument();
    expect(screen.getByText("B")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /Production/ }));
    expect(onSubmit).toHaveBeenCalledTimes(1);
    expect(onSubmit).toHaveBeenCalledWith("Production");
  });

  it("does not render a generic Submit button for choice questions", () => {
    render(<AskHumanInline request={makeRequest()} onSubmit={vi.fn()} onSkip={vi.fn()} />);
    expect(screen.queryByRole("button", { name: "Submit" })).not.toBeInTheDocument();
  });

  it("reveals a free-text field via 'Other' and submits the typed answer", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    render(<AskHumanInline request={makeRequest()} onSubmit={onSubmit} onSkip={vi.fn()} />);

    // No free-text field until the user opts into "Other".
    expect(screen.queryByPlaceholderText(/Type your own/)).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /Other/ }));
    await user.type(screen.getByPlaceholderText(/Type your own/), "10.0.0.5");
    await user.click(screen.getByRole("button", { name: "Send" }));

    expect(onSubmit).toHaveBeenCalledWith("10.0.0.5");
  });

  it("submits the 'Other' answer on Enter and trims surrounding whitespace", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    render(<AskHumanInline request={makeRequest()} onSubmit={onSubmit} onSkip={vi.fn()} />);

    await user.click(screen.getByRole("button", { name: /Other/ }));
    await user.type(screen.getByPlaceholderText(/Type your own/), "  custom answer  {Enter}");

    expect(onSubmit).toHaveBeenCalledWith("custom answer");
  });

  it("opens the free-text field immediately when a choice has no options", () => {
    render(
      <AskHumanInline request={makeRequest({ options: [] })} onSubmit={vi.fn()} onSkip={vi.fn()} />
    );
    expect(screen.getByPlaceholderText(/Type your own/)).toBeInTheDocument();
  });

  it("calls onSkip when Skip is clicked", async () => {
    const user = userEvent.setup();
    const onSkip = vi.fn();
    render(<AskHumanInline request={makeRequest()} onSubmit={vi.fn()} onSkip={onSkip} />);

    await user.click(screen.getByRole("button", { name: "Skip" }));
    expect(onSkip).toHaveBeenCalledTimes(1);
  });

  it("still supports plain free-text questions via the Submit button", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    render(
      <AskHumanInline
        request={makeRequest({ inputType: "freetext", options: [] })}
        onSubmit={onSubmit}
        onSkip={vi.fn()}
      />
    );

    await user.type(screen.getByPlaceholderText(/Type your response/), "hello");
    await user.click(screen.getByRole("button", { name: "Submit" }));
    expect(onSubmit).toHaveBeenCalledWith("hello");
  });

  it("renders selectable buttons when the model supplies options but left input_type at freetext", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    // Simulate the real flow: the event hook resolves the effective input_type.
    const inputType = resolveAskHumanInputType("freetext", ["Black-box", "Grey-box"]);
    render(
      <AskHumanInline
        request={makeRequest({ inputType, options: ["Black-box", "Grey-box"] })}
        onSubmit={onSubmit}
        onSkip={vi.fn()}
      />
    );

    await user.click(screen.getByRole("button", { name: /Black-box/ }));
    expect(onSubmit).toHaveBeenCalledWith("Black-box");
  });
});

describe("resolveAskHumanInputType", () => {
  it("coerces freetext to choice when options are present (model forgot to set choice)", () => {
    expect(resolveAskHumanInputType("freetext", ["A", "B"])).toBe("choice");
  });

  it("coerces a missing/unknown input_type to choice when options are present", () => {
    expect(resolveAskHumanInputType(undefined, ["A", "B"])).toBe("choice");
    expect(resolveAskHumanInputType(null, ["A"])).toBe("choice");
    expect(resolveAskHumanInputType("garbage", ["A"])).toBe("choice");
  });

  it("keeps freetext when there are no options", () => {
    expect(resolveAskHumanInputType("freetext", [])).toBe("freetext");
    expect(resolveAskHumanInputType(undefined, [])).toBe("freetext");
    expect(resolveAskHumanInputType("garbage", [])).toBe("freetext");
  });

  it("preserves an explicit choice", () => {
    expect(resolveAskHumanInputType("choice", ["A", "B"])).toBe("choice");
    expect(resolveAskHumanInputType("choice", [])).toBe("choice");
  });

  it("never overrides deliberate credentials/confirmation types", () => {
    expect(resolveAskHumanInputType("credentials", ["A"])).toBe("credentials");
    expect(resolveAskHumanInputType("confirmation", ["Yes", "No"])).toBe("confirmation");
  });
});

describe("parseReviewContext", () => {
  it("treats blank context as empty rows", () => {
    expect(parseReviewContext("")).toEqual({ kind: "rows", rows: [] });
    expect(parseReviewContext("   ")).toEqual({ kind: "rows", rows: [] });
  });

  it("extracts an organization_id (snake_case or camelCase) to source rows from the DB", () => {
    expect(parseReviewContext('{"organization_id":"org-1"}')).toEqual({
      kind: "org",
      organizationId: "org-1",
    });
    expect(parseReviewContext('{"organizationId":"  org-2  "}')).toEqual({
      kind: "org",
      organizationId: "org-2",
    });
  });

  it("tolerates a double-encoded JSON string carrying the org id", () => {
    expect(parseReviewContext(JSON.stringify('{"organization_id":"org-3"}'))).toEqual({
      kind: "org",
      organizationId: "org-3",
    });
  });

  it("passes a JSON array straight through as rows (back-compat)", () => {
    expect(parseReviewContext('[{"name":"Acme"}]')).toEqual({
      kind: "rows",
      rows: [{ name: "Acme" }],
    });
  });

  it("unwraps a wrapped array under items/candidates/units/organizations", () => {
    expect(parseReviewContext('{"items":[{"name":"A"}]}')).toEqual({
      kind: "rows",
      rows: [{ name: "A" }],
    });
    expect(parseReviewContext('{"organizations":[{"name":"B"}]}')).toEqual({
      kind: "rows",
      rows: [{ name: "B" }],
    });
  });

  it("falls back to bulk text for non-JSON or unrecognised payloads", () => {
    expect(parseReviewContext("Acme Corp\nAcme Sub")).toEqual({
      kind: "bulk",
      text: "Acme Corp\nAcme Sub",
    });
    expect(parseReviewContext('{"note":"why i need this"}')).toEqual({
      kind: "bulk",
      text: '{"note":"why i need this"}',
    });
  });
});

describe("AskHumanInline unit_review (DB-sourced candidates)", () => {
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

  function reviewRequest(context: string): AskHumanState {
    return {
      requestId: "req-u",
      sessionId: "sess-u",
      question: "Confirm the subsidiaries in scope",
      inputType: "unit_review",
      options: [],
      context,
    };
  }

  it("loads candidates from the DB by org id and seeds the table with ownership labels", async () => {
    vi.mocked(listOrganizationCandidates).mockResolvedValue({
      organizations: [
        candidate("cand-bank", "平安银行股份有限公司", { raw: { scale: "58%" } }),
        candidate("cand-securities", "平安证券股份有限公司"),
      ],
      targets: [],
    });

    render(
      <AskHumanInline
        request={reviewRequest('{"organization_id":"11111111-2222-3333-4444-555555555555"}')}
        onSubmit={vi.fn()}
        onSkip={vi.fn()}
      />
    );

    expect(await screen.findByLabelText("Name for unit 1")).toHaveValue("平安银行股份有限公司");
    expect(screen.getByLabelText("Name for unit 2")).toHaveValue("平安证券股份有限公司");
    expect(screen.getByText("58%")).toBeInTheDocument();
    expect(listOrganizationCandidates).toHaveBeenCalledWith(
      "11111111-2222-3333-4444-555555555555"
    );
  });

  it("submits stable unit identities in a UnitReviewSubmission envelope", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    vi.mocked(listOrganizationCandidates).mockResolvedValue({
      organizations: [
        candidate("cand-1", "Child A", {}, {
          organizationId: "org-1",
          ownershipPercent: "51%",
        }),
      ],
      targets: [],
    });

    render(
      <AskHumanInline
        request={reviewRequest('{"organization_id":"11111111-2222-3333-4444-555555555555"}')}
        onSubmit={onSubmit}
        onSkip={vi.fn()}
      />
    );

    await screen.findByLabelText("Name for unit 1");
    await user.click(screen.getByRole("button", { name: "Confirm" }));

    expect(onSubmit).toHaveBeenCalledTimes(1);
    expect(JSON.parse(onSubmit.mock.calls[0][0])).toEqual({
      rows: [
        expect.objectContaining({
          reviewRowId: "candidate:cand-1",
          candidateId: "cand-1",
          organizationId: "org-1",
          included: true,
        }),
      ],
    });
  });

  it("does not hit the DB and seeds from the array when context is a candidate array", () => {
    vi.mocked(listOrganizationCandidates).mockClear();
    render(
      <AskHumanInline
        request={reviewRequest('[{"name":"Acme Sub (51%)"}]')}
        onSubmit={vi.fn()}
        onSkip={vi.fn()}
      />
    );
    expect(screen.getByLabelText("Name for unit 1")).toHaveValue("Acme Sub (51%)");
    expect(listOrganizationCandidates).not.toHaveBeenCalled();
  });

  const FALLBACK = "140315f6-990e-4c5c-a04b-73b14310bf22";

  it("falls back to the discover org id when the model's context org_id is not a UUID", async () => {
    // Regression: mimo emits a placeholder/`"None"` org id, the DB fetch fails,
    // and the table silently empties. The captured discover org id rescues it.
    vi.mocked(listOrganizationCandidates).mockResolvedValue({
      organizations: [
        candidate("cand-bank", "平安银行股份有限公司", { raw: { scale: "58%" } }),
      ],
      targets: [],
    });
    render(
      <AskHumanInline
        request={reviewRequest('{"organization_id":"None"}')}
        onSubmit={vi.fn()}
        onSkip={vi.fn()}
        fallbackOrgId={FALLBACK}
      />
    );
    expect(await screen.findByLabelText("Name for unit 1")).toHaveValue("平安银行股份有限公司");
    expect(screen.getByText("58%")).toBeInTheDocument();
    expect(listOrganizationCandidates).toHaveBeenCalledWith(FALLBACK);
  });

  it("uses the discover org id when the context carries no org id at all", async () => {
    vi.mocked(listOrganizationCandidates).mockResolvedValue({
      organizations: [candidate("cand-securities", "平安证券股份有限公司")],
      targets: [],
    });
    render(
      <AskHumanInline
        request={reviewRequest("")}
        onSubmit={vi.fn()}
        onSkip={vi.fn()}
        fallbackOrgId={FALLBACK}
      />
    );
    expect(await screen.findByLabelText("Name for unit 1")).toHaveValue("平安证券股份有限公司");
    expect(listOrganizationCandidates).toHaveBeenCalledWith(FALLBACK);
  });

  it("prefers a valid context org id over the discover fallback", async () => {
    vi.mocked(listOrganizationCandidates).mockClear();
    vi.mocked(listOrganizationCandidates).mockResolvedValue({ organizations: [], targets: [] });
    render(
      <AskHumanInline
        request={reviewRequest('{"organization_id":"aaaaaaaa-1111-2222-3333-444444444444"}')}
        onSubmit={vi.fn()}
        onSkip={vi.fn()}
        fallbackOrgId={FALLBACK}
      />
    );
    await waitFor(() => {
      expect(listOrganizationCandidates).toHaveBeenCalledWith("aaaaaaaa-1111-2222-3333-444444444444");
    });
    expect(listOrganizationCandidates).not.toHaveBeenCalledWith(FALLBACK);
  });

  it("hides sub-threshold subsidiaries in unit_review when a discovery threshold is set", async () => {
    vi.mocked(listOrganizationCandidates).mockResolvedValue({
      organizations: [
        candidate("cand-bank", "平安银行股份有限公司", { raw: { scale: "58%" } }),
        candidate("cand-doctor", "平安好医生", { raw: { scale: "39%" } }),
      ],
      targets: [],
    });
    render(
      <AskHumanInline
        request={reviewRequest('{"organization_id":"11111111-2222-3333-4444-555555555555"}')}
        onSubmit={vi.fn()}
        onSkip={vi.fn()}
        minOwnershipPercent={51}
      />
    );
    expect(await screen.findByLabelText("Name for unit 1")).toHaveValue("平安银行股份有限公司");
    expect(screen.queryByDisplayValue("平安好医生")).not.toBeInTheDocument();
  });

  it("does not use the discover fallback for scope_review (only unit_review)", () => {
    vi.mocked(listOrganizationCandidates).mockClear();
    render(
      <AskHumanInline
        request={{ ...reviewRequest("pingan.com"), inputType: "scope_review" }}
        onSubmit={vi.fn()}
        onSkip={vi.fn()}
        fallbackOrgId={FALLBACK}
      />
    );
    expect(listOrganizationCandidates).not.toHaveBeenCalled();
  });
});

describe("AskHumanInline auto-confirm countdown", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.clearAllTimers();
    vi.useRealTimers();
  });

  function req(partial: Partial<AskHumanState> = {}): AskHumanState {
    return {
      requestId: "req-cd",
      sessionId: "sess-cd",
      question: "Proceed?",
      inputType: "confirmation",
      options: [],
      context: "",
      ...partial,
    };
  }

  // Drain the full countdown (plus a margin) inside act so the fire effect runs.
  const runOutClock = () => {
    act(() => {
      vi.advanceTimersByTime(ASK_HUMAN_COUNTDOWN_MS + 1000);
    });
  };

  it("renders an auto-confirm progress bar with a seconds label", () => {
    render(
      <AskHumanInline request={req()} onSubmit={vi.fn()} onSkip={vi.fn()} autoResolve />
    );
    expect(screen.getByText(/Auto-confirming in \d+s/)).toBeInTheDocument();
  });

  it("never auto-confirms while the approval mode is ask", () => {
    const onSubmit = vi.fn();
    render(<AskHumanInline request={req()} onSubmit={onSubmit} onSkip={vi.fn()} />);

    expect(screen.queryByText(/Auto-confirming in/)).not.toBeInTheDocument();
    runOutClock();
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("auto-submits the affirmative default ('yes') for a confirmation when the clock runs out", () => {
    const onSubmit = vi.fn();
    render(<AskHumanInline request={req()} onSubmit={onSubmit} onSkip={vi.fn()} autoResolve />);
    expect(onSubmit).not.toHaveBeenCalled();
    runOutClock();
    expect(onSubmit).toHaveBeenCalledTimes(1);
    expect(onSubmit).toHaveBeenCalledWith("yes");
  });

  it("auto-submits the first option for a choice when the clock runs out", () => {
    const onSubmit = vi.fn();
    render(
      <AskHumanInline
        request={req({ inputType: "choice", options: ["Production", "Staging"] })}
        onSubmit={onSubmit}
        onSkip={vi.fn()}
        autoResolve
      />
    );
    runOutClock();
    expect(onSubmit).toHaveBeenCalledTimes(1);
    expect(onSubmit).toHaveBeenCalledWith("Production");
  });

  it("never auto-confirms a subsidiary scope decision", () => {
    const onSubmit = vi.fn();
    const onSkip = vi.fn();
    render(
      <AskHumanInline
        request={req({
          inputType: "choice",
          options: ["不纳入子公司（仅母公司）", "纳入：≥51% 控股子公司"],
          context:
            '{"decision":"subsidiary_scope","organization_id":"11111111-2222-3333-4444-555555555555"}',
        })}
        onSubmit={onSubmit}
        onSkip={onSkip}
        autoResolve
      />
    );
    runOutClock();
    expect(onSubmit).not.toHaveBeenCalled();
    expect(onSkip).not.toHaveBeenCalled();
  });

  it("keeps an in-flight legacy subsidiary choice waiting too", () => {
    const onSubmit = vi.fn();
    render(
      <AskHumanInline
        request={req({
          inputType: "choice",
          question: "杭州默安科技有限公司是否纳入子公司？",
          options: ["不纳入子公司（仅母公司）", "纳入：≥51% 控股子公司"],
          context: "Subsidiary scope decision",
        })}
        onSubmit={onSubmit}
        onSkip={vi.fn()}
        autoResolve
      />
    );
    runOutClock();
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("keeps a choice with no selectable default waiting for explicit input", () => {
    const onSubmit = vi.fn();
    const onSkip = vi.fn();
    render(
      <AskHumanInline
        request={req({ inputType: "choice", options: [] })}
        onSubmit={onSubmit}
        onSkip={onSkip}
        autoResolve
      />
    );
    expect(screen.queryByText(/Auto-confirming in/)).not.toBeInTheDocument();
    runOutClock();
    expect(onSubmit).not.toHaveBeenCalled();
    expect(onSkip).not.toHaveBeenCalled();
  });

  it.each([
    ["scope_review", "[]"],
    ["unit_review", "[]"],
    ["credentials", ""],
    ["freetext", ""],
    ["unexpected", ""],
  ])("keeps %s requests waiting for an explicit human action", (inputType, context) => {
    const onSubmit = vi.fn();
    const onSkip = vi.fn();
    render(
      <AskHumanInline
        request={req({
          inputType: inputType as AskHumanState["inputType"],
          options: [],
          context,
        })}
        onSubmit={onSubmit}
        onSkip={onSkip}
        autoResolve
      />
    );

    expect(screen.queryByText(/Auto-confirming in/)).not.toBeInTheDocument();
    runOutClock();
    expect(onSubmit).not.toHaveBeenCalled();
    expect(onSkip).not.toHaveBeenCalled();
  });

  it.each(["freetext", "unexpected"])(
    "renders %s with options as a choice but never auto-selects its first option",
    (rawInputType) => {
      const onSubmit = vi.fn();
      render(
        <AskHumanInline
          request={req({
            rawInputType,
            inputType: "choice",
            options: ["Production", "Staging"],
          })}
          onSubmit={onSubmit}
          onSkip={vi.fn()}
          autoResolve
        />
      );

      expect(screen.getByRole("button", { name: /Production/ })).toBeInTheDocument();
      expect(screen.queryByText(/Auto-confirming in/)).not.toBeInTheDocument();
      runOutClock();
      expect(onSubmit).not.toHaveBeenCalled();
    }
  );

  it("never auto-confirms a phase-boundary confirmation even in run-all mode", () => {
    const onSubmit = vi.fn();
    render(
      <AskHumanInline
        request={req({
          inputType: "confirmation",
          question:
            "Approve entering the next phase (crossing target_intel → external_attack_surface)?",
          context: "Phase-boundary gate: Confirm to let the agent proceed.",
        })}
        onSubmit={onSubmit}
        onSkip={vi.fn()}
        autoResolve
      />
    );

    expect(screen.queryByText(/Auto-confirming in/)).not.toBeInTheDocument();
    runOutClock();
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("keeps review tables waiting for an explicit human decision", () => {
    const onSubmit = vi.fn();
    const onSkip = vi.fn();
    render(
      <AskHumanInline
        request={req({ inputType: "unit_review", options: [], context: '[{"name":"Acme Sub"}]' })}
        onSubmit={onSubmit}
        onSkip={onSkip}
        autoResolve
      />
    );
    expect(screen.getByText("Waiting for your review")).toBeInTheDocument();
    runOutClock();
    expect(onSubmit).not.toHaveBeenCalled();
    expect(onSkip).not.toHaveBeenCalled();
  });

  it("fires the default action only once even past the deadline", () => {
    const onSubmit = vi.fn();
    render(<AskHumanInline request={req()} onSubmit={onSubmit} onSkip={vi.fn()} autoResolve />);
    runOutClock();
    runOutClock();
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });

  it("pauses while hovered and resumes (then fires) on mouse leave", () => {
    const onSubmit = vi.fn();
    const { container } = render(
      <AskHumanInline request={req()} onSubmit={onSubmit} onSkip={vi.fn()} autoResolve />
    );
    const box = container.firstChild as HTMLElement;

    act(() => {
      vi.advanceTimersByTime(ASK_HUMAN_COUNTDOWN_MS / 2);
    });
    act(() => {
      fireEvent.mouseEnter(box);
    });
    // Hovering freezes the clock: the box shows a paused hint and never submits.
    expect(screen.getByText(/Paused/)).toBeInTheDocument();
    act(() => {
      vi.advanceTimersByTime(ASK_HUMAN_COUNTDOWN_MS * 2);
    });
    expect(onSubmit).not.toHaveBeenCalled();

    act(() => {
      fireEvent.mouseLeave(box);
    });
    runOutClock();
    expect(onSubmit).toHaveBeenCalledTimes(1);
    expect(onSubmit).toHaveBeenCalledWith("yes");
  });
});

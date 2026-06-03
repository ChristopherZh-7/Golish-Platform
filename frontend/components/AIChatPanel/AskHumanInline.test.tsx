import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { AskHumanInline, type AskHumanState, resolveAskHumanInputType } from "./AskHumanInline";

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

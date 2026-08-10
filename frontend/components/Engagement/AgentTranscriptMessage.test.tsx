import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { AgentTranscriptMessage } from "./AgentTranscriptMessage";

describe("AgentTranscriptMessage", () => {
  it("renders assistant prose as compact Markdown instead of exposing syntax", () => {
    render(
      <AgentTranscriptMessage
        kind="text"
        actorLabel="漏洞扫描调度器"
        timeLabel="09:39:52"
        text={"DB truth:\n\n- **Worklist** is ready\n- use `ready_to_submit=true`"}
      />
    );

    const message = screen.getByTestId("agent-transcript-message");
    expect(message).toHaveTextContent("Worklist");
    expect(message).not.toHaveTextContent("**Worklist**");
    expect(screen.getByText("Worklist").tagName).toBe("STRONG");
    expect(screen.getByText("ready_to_submit=true").tagName).toBe("CODE");
    expect(message.querySelectorAll("li")).toHaveLength(2);
  });

  it("keeps settled thinking collapsed to a one-line duration until requested", () => {
    render(
      <AgentTranscriptMessage
        kind="thinking"
        actorLabel="Verifier"
        text="Private reasoning detail"
        startedAt={1_000}
        endedAt={2_200}
      />
    );

    const toggle = screen.getByRole("button", { name: "Thought for 1.2s" });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("Private reasoning detail")).not.toBeInTheDocument();

    fireEvent.click(toggle);
    expect(screen.getByText("Private reasoning detail")).toBeInTheDocument();
  });

  it("shows active thinking, then automatically collapses the settled thought", () => {
    const view = render(
      <AgentTranscriptMessage
        kind="thinking"
        actorLabel="Verifier"
        text="Streaming private reasoning"
        startedAt={1_000}
        endedAt={2_200}
        thinkingActive
      />
    );

    expect(screen.getByRole("button", { name: "Thinking" })).toHaveAttribute(
      "aria-expanded",
      "true"
    );
    expect(screen.getByText("Streaming private reasoning")).toBeInTheDocument();

    view.rerender(
      <AgentTranscriptMessage
        kind="thinking"
        actorLabel="Verifier"
        text="Streaming private reasoning"
        startedAt={1_000}
        endedAt={2_200}
      />
    );

    expect(screen.getByRole("button", { name: "Thought for 1.2s" })).toHaveAttribute(
      "aria-expanded",
      "false"
    );
    expect(screen.queryByText("Streaming private reasoning")).not.toBeInTheDocument();
  });
});

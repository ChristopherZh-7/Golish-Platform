import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ThinkingBlock } from "./ThinkingBlock";

describe("ThinkingBlock duration", () => {
  it("does not invent a 0.001 second duration for a zero-width batch", () => {
    render(
      <ThinkingBlock content="done" isActive={false} startedAt={1_000} endedAt={1_000} />
    );

    expect(screen.getByRole("button", { name: "Thought" })).toBeInTheDocument();
    expect(screen.queryByText(/0\.001s/)).not.toBeInTheDocument();
  });

  it("labels a measured sub-100ms segment without fake millisecond precision", () => {
    render(
      <ThinkingBlock content="done" isActive={false} startedAt={1_000} endedAt={1_040} />
    );

    expect(screen.getByRole("button", { name: "Thought for <0.1s" })).toBeInTheDocument();
  });
});

import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { BackgroundJobsBadge } from "./StatusBadges";

describe("BackgroundJobsBadge", () => {
  it("renders a fallback count when background job details are unavailable", () => {
    render(<BackgroundJobsBadge jobs={[]} fallbackCount={2} />);

    expect(screen.getByRole("button", { name: /2 running/i })).toBeInTheDocument();
  });
});

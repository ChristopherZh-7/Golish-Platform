import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { WhatWebAssessmentBadge } from "./WhatWebAssessmentBadge";

describe("WhatWebAssessmentBadge", () => {
  it("shows an unassessed origin without treating it as clean", () => {
    render(<WhatWebAssessmentBadge assessment={null} />);
    expect(screen.getByText("Not assessed")).toBeInTheDocument();
    expect(screen.getByTitle(/No authoritative WhatWeb evidence/)).toBeInTheDocument();
  });

  it("shows producer blocked without claiming downstream exclusion", () => {
    render(
      <WhatWebAssessmentBadge
        assessment={{
          origin: "http://123.6.40.244:8000",
          state: "producer_blocked",
          observedAt: "2026-07-17T01:02:06Z",
          attempt: 3,
          failureClass: "connection_reset",
        }}
      />
    );
    expect(screen.getByText("WhatWeb stopped")).toBeInTheDocument();
    expect(screen.queryByText(/Excluded/i)).not.toBeInTheDocument();
    expect(screen.getByTitle(/Enumeration decides exact-origin eligibility/)).toBeInTheDocument();
  });
});

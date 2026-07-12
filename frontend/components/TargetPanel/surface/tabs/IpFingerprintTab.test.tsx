import { render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { Fingerprint } from "@/lib/security-analysis";
import { IpFingerprintTab } from "./IpFingerprintTab";

const legacyFingerprint: Fingerprint = {
  id: "legacy-nginx",
  targetId: "target-ip",
  projectPath: null,
  category: "webserver",
  name: "nginx",
  version: "1.24",
  confidence: 0.8,
  evidence: [{ source: "whatweb", raw: "nginx/1.24" }],
  cpe: null,
  source: "whatweb",
  detectedAt: "2026-07-12T00:00:00Z",
};

describe("IpFingerprintTab", () => {
  it("keeps a legacy no-origin fingerprint visible in the target-level unassigned section", () => {
    render(
      <IpFingerprintTab fingerprints={[legacyFingerprint]} webOrigins={[]} loading={false} />
    );

    const unassignedHeading = screen.getByText("Target-level / unassigned fingerprints");
    const unassignedSection = unassignedHeading.closest("section");
    expect(unassignedSection).not.toBeNull();
    expect(within(unassignedSection as HTMLElement).getByText("nginx")).toBeInTheDocument();
    expect(screen.getByText(/legacy evidence without an origin stays target-level/)).toBeInTheDocument();
  });
});

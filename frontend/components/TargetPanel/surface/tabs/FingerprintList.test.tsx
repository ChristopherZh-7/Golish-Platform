import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { Fingerprint } from "@/lib/security-analysis";
import { FingerprintList } from "./FingerprintList";

const fp = (over: Partial<Fingerprint> = {}): Fingerprint => ({
  id: "fp-1",
  targetId: "target-1",
  projectPath: null,
  category: "server",
  name: "nginx",
  version: "1.25.3",
  confidence: 90,
  evidence: [],
  cpe: "cpe:/a:nginx:nginx:1.25.3",
  source: "whatweb",
  detectedAt: "2026-07-02T00:00:00Z",
  ...over,
});

describe("FingerprintList", () => {
  it("renders detected fingerprints with name, version, category and cpe", () => {
    render(<FingerprintList fingerprints={[fp()]} />);
    expect(screen.getByText("nginx")).toBeInTheDocument();
    expect(screen.getByText("1.25.3")).toBeInTheDocument();
    expect(screen.getByText("server")).toBeInTheDocument();
    expect(screen.getByText("cpe:/a:nginx:nginx:1.25.3")).toBeInTheDocument();
    expect(screen.getByText("1 detected")).toBeInTheDocument();
  });

  it("shows empty state when no fingerprints", () => {
    render(<FingerprintList fingerprints={[]} />);
    expect(screen.getByText(/No service\/version fingerprint/)).toBeInTheDocument();
  });

  it("honors a custom empty label", () => {
    render(<FingerprintList fingerprints={[]} emptyLabel="custom empty" />);
    expect(screen.getByText("custom empty")).toBeInTheDocument();
  });

  it("omits version when absent", () => {
    render(<FingerprintList fingerprints={[fp({ version: null })]} />);
    expect(screen.queryByText("1.25.3")).not.toBeInTheDocument();
    expect(screen.getByText("nginx")).toBeInTheDocument();
  });

  it("omits cpe row when absent", () => {
    render(<FingerprintList fingerprints={[fp({ cpe: null })]} />);
    expect(screen.queryByText(/cpe:/)).not.toBeInTheDocument();
  });

  it("truncates to the limit and reports overflow", () => {
    const list = Array.from({ length: 12 }, (_, i) => fp({ id: `fp-${i}`, name: `svc-${i}` }));
    render(<FingerprintList fingerprints={list} limit={10} />);
    expect(screen.getByText("svc-0")).toBeInTheDocument();
    expect(screen.queryByText("svc-10")).not.toBeInTheDocument();
    expect(screen.getByText("+2 more")).toBeInTheDocument();
  });
});

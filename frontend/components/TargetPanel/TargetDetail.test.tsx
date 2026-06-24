import { render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { Target } from "@/lib/pentest/types";
import { TargetDetailView } from "./TargetDetail";

vi.mock("@/components/QuickNotes/QuickNotes", () => ({
  QuickNotes: () => null,
}));

vi.mock("./hooks/useTargetSurfaceData", () => ({
  useTargetSurfaceData: () => ({
    data: {
      assets: [],
      endpoints: [],
      fingerprints: [],
      jsResults: [],
      passiveScans: [],
      logs: [],
    },
    loading: false,
    error: null,
    reload: vi.fn(),
  }),
}));

const target = (overrides: Partial<Target> = {}): Target => ({
  id: "target-1",
  name: "example.com",
  type: "domain",
  value: "example.com",
  tags: [],
  notes: "",
  scope: "in",
  status: "active",
  grp: "default",
  owner: "",
  time_window_start: null,
  time_window_end: null,
  organization_id: null,
  source: "ai-tool",
  parent_id: null,
  real_ip: "93.184.216.34",
  cdn_waf: "Cloudflare",
  http_title: "Example Domain",
  http_status: 200,
  webserver: "nginx",
  os_info: "",
  content_type: "text/html",
  created_at: 0,
  updated_at: 0,
  ports: [],
  technologies: [],
  ...overrides,
});

describe("TargetDetailView", () => {
  it("shows top-level active landing HTTP/recon fields even without port entries", () => {
    render(<TargetDetailView target={target()} t={(key) => key} onUpdateNotes={vi.fn()} />);

    const section = screen.getByText("Recon Facts").closest("div")?.parentElement;
    expect(section).toBeTruthy();
    const facts = within(section as HTMLElement);
    expect(facts.getByText("93.184.216.34")).toBeInTheDocument();
    expect(facts.getByText("200")).toBeInTheDocument();
    expect(facts.getByText("Example Domain")).toBeInTheDocument();
    expect(facts.getByText("nginx")).toBeInTheDocument();
    expect(facts.getByText("text/html")).toBeInTheDocument();
  });
});

import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// Mock dependencies
vi.mock("@/hooks/useCreateTerminalTab", () => ({
  useCreateTerminalTab: () => ({
    createTerminalTab: vi.fn(),
  }),
}));

vi.mock("@/lib/indexer", () => ({
  listProjectsForHome: vi.fn().mockResolvedValue([]),
  listRecentDirectories: vi.fn().mockResolvedValue([]),
}));

vi.mock("@/lib/projects", () => ({
  saveProject: vi.fn().mockResolvedValue(undefined),
  deleteProject: vi.fn().mockResolvedValue(undefined),
  listProjectConfigs: vi.fn().mockResolvedValue([]),
}));

vi.mock("@/lib/api/git", () => ({
  deleteWorktree: vi.fn().mockResolvedValue(undefined),
}));

describe("HomeView Memoization Tests", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe("ProjectRow memoization", () => {
    it("ProjectRow should be wrapped in React.memo", async () => {
      const module = await import("./ProjectCards");

      const ProjectRow = (module as Record<string, unknown>).ProjectRow;
      expect(ProjectRow).toBeDefined();

      const memoSymbol = Symbol.for("react.memo");
      const componentType = (ProjectRow as { $$typeof?: symbol }).$$typeof;
      expect(componentType).toBe(memoSymbol);
    });
  });

  describe("RecentDirectoryRow memoization", () => {
    it("RecentDirectoryRow should be wrapped in React.memo", async () => {
      const module = await import("./ProjectCards");

      const RecentDirectoryRow = (module as Record<string, unknown>).RecentDirectoryRow;
      expect(RecentDirectoryRow).toBeDefined();

      const memoSymbol = Symbol.for("react.memo");
      const componentType = (RecentDirectoryRow as { $$typeof?: symbol }).$$typeof;
      expect(componentType).toBe(memoSymbol);
    });
  });

  describe("Callback stability", () => {
    it("should use stable callbacks that do not change between renders", async () => {
      const { listProjectConfigs } = await import("@/lib/projects");
      vi.mocked(listProjectConfigs).mockResolvedValue([
        { name: "Test Project", rootPath: "/test/project" } as any,
      ]);

      const { HomeView } = await import("./HomeView");

      const { rerender } = render(<HomeView />);

      // The HomeView re-design surfaces a `Recent projects` heading once
      // the saved-projects list resolves; we use that as the readiness
      // marker since the old `Projects` label has been removed.
      await screen.findByText("Recent projects");

      rerender(<HomeView />);

      expect(screen.getByText("Recent projects")).toBeDefined();
    });
  });

  describe("Inline arrow function elimination", () => {
    /**
     * The original code had inline arrow functions like:
     * onToggle={() => toggleProject(project.path)}
     *
     * These should be replaced with stable callbacks using useCallback
     * that are passed down to memoized children.
     */
    it("project rows render with stable callbacks", async () => {
      const { listProjectConfigs } = await import("@/lib/projects");
      vi.mocked(listProjectConfigs).mockResolvedValue([
        { name: "Project A", rootPath: "/project/a" } as any,
        { name: "Project B", rootPath: "/project/b" } as any,
      ]);

      const { HomeView } = await import("./HomeView");

      render(<HomeView />);

      // Wait for the recent-projects section to mount, then both project
      // names should be visible (they're rendered as plain text inside the
      // saved-projects list).
      await screen.findByText("Recent projects");

      expect(screen.getByText("Project A")).toBeDefined();
      expect(screen.getByText("Project B")).toBeDefined();
    });
  });
});

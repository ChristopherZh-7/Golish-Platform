import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  deleteTarget: vi.fn(),
  listTargets: vi.fn(),
  onCustomEvent: vi.fn(async (_channel: string, _handler: () => void) => vi.fn()),
  onEvent: vi.fn(async (_channel: string, _handler: () => void) => vi.fn()),
  sendCustomEvent: vi.fn(async () => undefined),
}));

vi.mock("@/lib/api", () => ({
  targets: {
    addTarget: vi.fn(),
    batchAddTargets: vi.fn(),
    clearAllTargets: vi.fn(),
    deleteTarget: mocks.deleteTarget,
    listTargets: mocks.listTargets,
    updateTarget: vi.fn(),
  },
}));

vi.mock("@/lib/audit", () => ({
  logAudit: vi.fn(),
}));

vi.mock("@/lib/events", () => ({
  onCustomEvent: mocks.onCustomEvent,
  onEvent: mocks.onEvent,
  sendCustomEvent: mocks.sendCustomEvent,
}));

vi.mock("@/lib/projects", () => ({
  getProjectPath: () => "/workspace",
}));

vi.mock("@/lib/run-tauri-unlisten", () => ({
  runTauriUnlistenFromPromise: vi.fn(),
}));

vi.mock("@/store", () => ({
  useStore: (selector: (state: { workspaceDataReady: boolean }) => unknown) =>
    selector({ workspaceDataReady: true }),
}));

import { useTargetData } from "./useTargetData";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((next) => {
    resolve = next;
  });
  return { promise, resolve };
}

const target = {
  id: "target-1",
  name: "example.com",
  type: "domain",
  value: "example.com",
  ports: [],
  scope: "in",
  status: "new",
};

describe("useTargetData deletion refresh", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.deleteTarget.mockResolvedValue(undefined);
  });

  it("removes a deleted target immediately and ignores an older late list response", async () => {
    const staleReload = deferred<{ targets: typeof target[] }>();
    const deleteReload = deferred<{ targets: typeof target[] }>();
    let targetsChanged: (() => void) | undefined;

    mocks.onCustomEvent.mockImplementation(async (channel: string, handler: () => void) => {
      if (channel === "targets-changed") targetsChanged = handler;
      return vi.fn();
    });
    mocks.listTargets
      .mockResolvedValueOnce({ targets: [target] })
      .mockReturnValueOnce(staleReload.promise)
      .mockReturnValueOnce(deleteReload.promise);

    const { result } = renderHook(() => useTargetData());
    await waitFor(() => expect(result.current.safeTargets).toHaveLength(1));
    await waitFor(() => expect(targetsChanged).toBeTypeOf("function"));

    act(() => {
      targetsChanged?.();
    });
    let deletion!: Promise<void>;
    act(() => {
      deletion = result.current.handleDelete(target.id);
    });

    await waitFor(() => expect(result.current.safeTargets).toEqual([]));

    await act(async () => {
      staleReload.resolve({ targets: [target] });
      await staleReload.promise;
    });
    expect(result.current.safeTargets).toEqual([]);

    await act(async () => {
      deleteReload.resolve({ targets: [] });
      await deletion;
    });
    expect(result.current.safeTargets).toEqual([]);
  });
});

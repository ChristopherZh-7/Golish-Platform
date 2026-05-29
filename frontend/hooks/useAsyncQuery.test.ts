import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { useAsyncQuery } from "./useAsyncQuery";

describe("useAsyncQuery", () => {
  it("transitions from loading to data on success", async () => {
    const fn = vi.fn().mockResolvedValue("hello");
    const { result } = renderHook(() => useAsyncQuery(fn, []));

    expect(result.current.loading).toBe(true);
    expect(result.current.data).toBeUndefined();

    await waitFor(() => expect(result.current.loading).toBe(false));

    expect(result.current.data).toBe("hello");
    expect(result.current.error).toBeNull();
    expect(fn).toHaveBeenCalledTimes(1);
  });

  it("captures the error message on failure", async () => {
    const fn = vi.fn().mockRejectedValue(new Error("boom"));
    const { result } = renderHook(() => useAsyncQuery(fn, []));

    await waitFor(() => expect(result.current.loading).toBe(false));

    expect(result.current.error).toBe("boom");
    expect(result.current.data).toBeUndefined();
  });

  it("re-runs the same fetch on reload", async () => {
    const fn = vi.fn().mockResolvedValueOnce("first").mockResolvedValueOnce("second");
    const { result } = renderHook(() => useAsyncQuery(fn, []));

    await waitFor(() => expect(result.current.data).toBe("first"));
    expect(fn).toHaveBeenCalledTimes(1);

    act(() => {
      result.current.reload();
    });

    await waitFor(() => expect(result.current.data).toBe("second"));
    expect(fn).toHaveBeenCalledTimes(2);
  });

  it("re-runs when deps change", async () => {
    const fn = vi.fn().mockResolvedValue("ok");
    const { rerender } = renderHook(({ id }: { id: number }) => useAsyncQuery(fn, [id]), {
      initialProps: { id: 1 },
    });

    await waitFor(() => expect(fn).toHaveBeenCalledTimes(1));

    rerender({ id: 2 });

    await waitFor(() => expect(fn).toHaveBeenCalledTimes(2));
  });

  it("stays idle when disabled and exposes initialData", () => {
    const fn = vi.fn().mockResolvedValue("nope");
    const { result } = renderHook(() =>
      useAsyncQuery(fn, [], { enabled: false, initialData: "seed" })
    );

    expect(result.current.loading).toBe(false);
    expect(result.current.data).toBe("seed");
    expect(fn).not.toHaveBeenCalled();
  });
});

import { useCallback, useEffect, useRef, useState } from "react";

export interface UseAsyncQueryOptions<T> {
  /** When false the query stays idle: `fn` never runs and `loading` is false. */
  enabled?: boolean;
  /** Value exposed as `data` before the first successful resolve. */
  initialData?: T;
}

export interface UseAsyncQueryResult<T> {
  data: T | undefined;
  loading: boolean;
  error: string | null;
  reload: () => void;
}

/**
 * Generic tri-state async data hook. Runs `fn` on mount and whenever `deps`
 * change, exposing `{ data, loading, error }` plus `reload` which re-runs the
 * exact same fetch path (no duplicated request logic). A per-run cancelled flag
 * keeps a stale resolve from clobbering fresher state across the
 * mount / deps-change / reload races.
 */
export function useAsyncQuery<T>(
  fn: () => Promise<T>,
  deps: unknown[],
  opts: UseAsyncQueryOptions<T> = {}
): UseAsyncQueryResult<T> {
  const { enabled = true, initialData } = opts;
  const [data, setData] = useState<T | undefined>(initialData);
  const [loading, setLoading] = useState<boolean>(enabled);
  const [error, setError] = useState<string | null>(null);
  const [reloadToken, setReloadToken] = useState(0);

  // Always invoke the latest `fn` without forcing callers to memoize it.
  const fnRef = useRef(fn);
  fnRef.current = fn;

  useEffect(() => {
    if (!enabled) {
      setLoading(false);
      return;
    }

    let cancelled = false;
    setLoading(true);
    setError(null);

    fnRef
      .current()
      .then((result) => {
        if (!cancelled) setData(result);
      })
      .catch((err) => {
        if (!cancelled) setError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [enabled, reloadToken, ...deps]);

  const reload = useCallback(() => {
    setReloadToken((token) => token + 1);
  }, []);

  return { data, loading, error, reload };
}

import { useCallback, useEffect, useRef, useState } from "react";

const STORAGE_KEY = "golish.unifiedInput.maxHeight";
const DEFAULT_MAX_HEIGHT = 200;
const MIN_MAX_HEIGHT = 80;
const MAX_MAX_HEIGHT = 800;

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

function readStored(): number {
  if (typeof window === "undefined") return DEFAULT_MAX_HEIGHT;
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return DEFAULT_MAX_HEIGHT;
    const parsed = Number(raw);
    if (!Number.isFinite(parsed)) return DEFAULT_MAX_HEIGHT;
    return clamp(parsed, MIN_MAX_HEIGHT, MAX_MAX_HEIGHT);
  } catch {
    return DEFAULT_MAX_HEIGHT;
  }
}

/**
 * Manages the resizable max-height of the UnifiedInput textarea. The user
 * drags the top edge of the input panel up/down to enlarge the textarea
 * when composing long commands. The chosen value is persisted to
 * localStorage so it survives reloads.
 *
 * Works identically on macOS and Windows because it relies purely on
 * pointer events + CSS — no native window APIs.
 */
export function useInputResize() {
  const [maxHeight, setMaxHeight] = useState<number>(() => readStored());
  const maxHeightRef = useRef(maxHeight);
  maxHeightRef.current = maxHeight;

  useEffect(() => {
    if (typeof window === "undefined") return;
    try {
      window.localStorage.setItem(STORAGE_KEY, String(maxHeight));
    } catch {
      // localStorage may be disabled (e.g. strict privacy mode); ignore.
    }
  }, [maxHeight]);

  useEffect(() => {
    return () => {
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };
  }, []);

  const handlePointerDown = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const startY = e.clientY;
    const startHeight = maxHeightRef.current;
    const pointerId = e.pointerId;

    const onMove = (ev: PointerEvent) => {
      if (ev.pointerId !== pointerId) return;
      const delta = startY - ev.clientY;
      const next = clamp(startHeight + delta, MIN_MAX_HEIGHT, MAX_MAX_HEIGHT);
      setMaxHeight(next);
    };
    const onUp = (ev: PointerEvent) => {
      if (ev.pointerId !== pointerId) return;
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onUp);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };

    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onUp);
    document.body.style.cursor = "ns-resize";
    document.body.style.userSelect = "none";
  }, []);

  const resetToDefault = useCallback(() => {
    setMaxHeight(DEFAULT_MAX_HEIGHT);
  }, []);

  return { maxHeight, handlePointerDown, resetToDefault };
}

import { useCallback, useEffect, useRef, useState } from "react";

const STORAGE_KEY = "golish.unifiedInput.desiredHeight";

const UNSET = 0; // 0 = user has never dragged; legacy auto-grow behavior applies
const MIN_USER_HEIGHT = 80;
const MAX_USER_HEIGHT = 800;
const IMPLICIT_START = 200; // legacy cap, used as the starting point when user first drags

function clampUser(value: number): number {
  return Math.max(MIN_USER_HEIGHT, Math.min(MAX_USER_HEIGHT, value));
}

function readStored(): number {
  if (typeof window === "undefined") return UNSET;
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return UNSET;
    const parsed = Number(raw);
    if (!Number.isFinite(parsed) || parsed <= 0) return UNSET;
    return clampUser(parsed);
  } catch {
    return UNSET;
  }
}

/**
 * Resizable command input panel.
 *
 * `desiredHeight` is the height the user dragged the top handle to. The
 * textarea will be **at least** this tall (and at most 800px) once the
 * user has interacted with the handle; before that, the textarea keeps
 * the legacy auto-grow behavior (one line by default, growing up to 200px).
 *
 * The value is persisted to localStorage so the chosen height survives
 * reloads. Works identically on macOS and Windows because it relies on
 * pointer events + CSS only.
 */
export function useInputResize() {
  const [desiredHeight, setDesiredHeight] = useState<number>(() => readStored());
  const desiredHeightRef = useRef(desiredHeight);
  desiredHeightRef.current = desiredHeight;

  useEffect(() => {
    if (typeof window === "undefined") return;
    try {
      if (desiredHeight === UNSET) {
        window.localStorage.removeItem(STORAGE_KEY);
      } else {
        window.localStorage.setItem(STORAGE_KEY, String(desiredHeight));
      }
    } catch {
      // localStorage may be disabled (e.g. strict privacy mode); ignore.
    }
  }, [desiredHeight]);

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
    const startHeight =
      desiredHeightRef.current === UNSET ? IMPLICIT_START : desiredHeightRef.current;
    const pointerId = e.pointerId;

    const onMove = (ev: PointerEvent) => {
      if (ev.pointerId !== pointerId) return;
      const delta = startY - ev.clientY;
      setDesiredHeight(clampUser(startHeight + delta));
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
    setDesiredHeight(UNSET);
  }, []);

  return { desiredHeight, handlePointerDown, resetToDefault };
}

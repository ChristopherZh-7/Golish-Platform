/**
 * Scrollbar auto-hide — emulates the macOS overlay-scrollbar fade behaviour
 * on platforms where the system scrollbar is permanently visible.
 *
 * How it works:
 *   - The CSS in `index.css` paints `::-webkit-scrollbar-thumb` transparent
 *     by default. Two CSS rules reveal it:
 *       1. `*:hover` — pointer hovers a scroll container.
 *       2. `html[data-scrolling] *` — this attribute, toggled here by JS,
 *          flashes every thumb in the document for a brief window after
 *          the most recent scroll event (matching macOS behaviour when
 *          the user scrolls without moving the pointer).
 *   - We listen to `scroll` in the capture phase because scroll events do
 *     not bubble — only capture sees nested scroll containers.
 *
 * Cross-platform notes:
 *   - macOS WKWebView ignores `::-webkit-scrollbar` pseudo-elements and
 *     keeps the system overlay; the attribute we toggle here has no
 *     observable effect there, so this code is safe to run unconditionally.
 *   - Windows WebView2 (Chromium) and Linux WebKitGTK both honour the CSS
 *     and will fade thumbs in/out.
 *   - Firefox styles via `scrollbar-color` (no pseudo-elements); the auto-
 *     fade isn't possible there, but the thumb is already thin enough.
 *
 * Safe to import multiple times — `init` is idempotent.
 */

const ROOT_ATTR = "data-scrolling";
const HIDE_DELAY_MS = 800;

let installed = false;
let hideTimer: ReturnType<typeof setTimeout> | null = null;

function flashScrollbars(): void {
  const root = document.documentElement;
  if (!root.hasAttribute(ROOT_ATTR)) {
    root.setAttribute(ROOT_ATTR, "");
  }
  if (hideTimer !== null) {
    clearTimeout(hideTimer);
  }
  hideTimer = setTimeout(() => {
    root.removeAttribute(ROOT_ATTR);
    hideTimer = null;
  }, HIDE_DELAY_MS);
}

export function installScrollbarAutoHide(): void {
  if (installed) return;
  if (typeof document === "undefined") return;
  installed = true;

  document.addEventListener("scroll", flashScrollbars, {
    capture: true,
    passive: true,
  });

  // A wheel event implies imminent scrolling on a scrollable container,
  // even when the cursor is over a non-scrollable child. Flashing the
  // scrollbar here gives slightly nicer feedback at the very start of
  // the gesture (before scrollTop has actually moved).
  document.addEventListener("wheel", flashScrollbars, {
    capture: true,
    passive: true,
  });
}

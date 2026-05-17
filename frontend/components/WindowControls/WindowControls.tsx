import { getCurrentWindow } from "@tauri-apps/api/window";
import { isWindows } from "@/lib/env";
import { isMockBrowserMode } from "@/mocks";

function handleMinimize() {
  getCurrentWindow().minimize();
}

function handleMaximize() {
  getCurrentWindow().toggleMaximize();
}

function handleClose() {
  getCurrentWindow().close();
}

const btnBase =
  "inline-flex items-center justify-center w-[46px] h-[32px] transition-colors duration-100 titlebar-no-drag";

export function WindowControls() {
  if (!isWindows() || isMockBrowserMode()) return null;

  return (
    <div className="flex items-center flex-shrink-0 h-[32px]">
      <button
        type="button"
        className={`${btnBase} hover:bg-white/10`}
        onClick={handleMinimize}
        aria-label="Minimize"
      >
        <svg
          width="10"
          height="1"
          viewBox="0 0 10 1"
          className="fill-current text-foreground/80"
          aria-hidden="true"
          focusable="false"
        >
          <title>Minimize</title>
          <rect width="10" height="1" />
        </svg>
      </button>

      <button
        type="button"
        className={`${btnBase} hover:bg-white/10`}
        onClick={handleMaximize}
        aria-label="Maximize"
      >
        <svg
          width="10"
          height="10"
          viewBox="0 0 10 10"
          className="stroke-current text-foreground/80"
          fill="none"
          aria-hidden="true"
          focusable="false"
        >
          <title>Maximize</title>
          <rect x="0.5" y="0.5" width="9" height="9" strokeWidth="1" />
        </svg>
      </button>

      <button
        type="button"
        className={`${btnBase} hover:bg-red-600`}
        onClick={handleClose}
        aria-label="Close"
      >
        <svg
          width="10"
          height="10"
          viewBox="0 0 10 10"
          className="stroke-current text-foreground/80"
          aria-hidden="true"
          focusable="false"
        >
          <title>Close</title>
          <line x1="0" y1="0" x2="10" y2="10" strokeWidth="1.2" />
          <line x1="10" y1="0" x2="0" y2="10" strokeWidth="1.2" />
        </svg>
      </button>
    </div>
  );
}

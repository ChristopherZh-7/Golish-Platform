import React from "react";
import ReactDOM from "react-dom/client";
import "./index.css";
import "./lib/i18n";
import { isTauri } from "@/lib/env";

function getDetachedParams(): { sessionId: string; tabType: string } | null {
  const params = new URLSearchParams(window.location.search);
  if (params.get("detached") !== "true") return null;
  const sessionId = params.get("session");
  const tabType = params.get("type") || "terminal";
  if (!sessionId) return null;
  return { sessionId, tabType };
}

async function initApp(): Promise<void> {
  if (!isTauri()) {
    console.log("[App] Running in browser mode - loading Tauri IPC mocks");
    const { setupMocks } = await import("./mocks");
    setupMocks();
  }

  const { setupGlobalErrorHandlers, ErrorBoundary } = await import("./components/ErrorBoundary");
  setupGlobalErrorHandlers();

  // Dev-only design previews (?preview=<name>): render a single prototype view
  // in the real app theme via `just dev-fe`, before wiring to the live backend.
  // Stripped from production via the DEV guard.
  if (import.meta.env.DEV) {
    const previewName = new URLSearchParams(window.location.search).get("preview");
    if (previewName === "intel" || previewName === "stage-run") {
      const [{ StageRunPreviewView }, { ThemeProvider }] = await Promise.all([
        import("./components/Engagement/StageRun.preview"),
        import("./hooks/useTheme"),
      ]);
      ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
        <React.StrictMode>
          <ErrorBoundary>
            <ThemeProvider defaultThemeId="golish">
              <StageRunPreviewView />
            </ThemeProvider>
          </ErrorBoundary>
        </React.StrictMode>
      );
      return;
    }
  }

  const detached = getDetachedParams();

  if (detached) {
    const { DetachedView } = await import("./components/DetachedView/DetachedView");
    ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
      <React.StrictMode>
        <ErrorBoundary>
          <DetachedView sessionId={detached.sessionId} tabType={detached.tabType} />
        </ErrorBoundary>
      </React.StrictMode>
    );
    return;
  }

  const { default: App } = await import("./App");
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <ErrorBoundary>
        <App />
      </ErrorBoundary>
    </React.StrictMode>
  );
}

initApp();

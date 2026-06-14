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

import React from "react";
import ReactDOM from "react-dom/client";
import { I18nextProvider } from "react-i18next";
import "./index.css";
import { isTauri } from "@/lib/env";
import i18n from "./lib/i18n";
import { installScrollbarAutoHide } from "./lib/scrollbar-autohide";

installScrollbarAutoHide();

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

  const { installZapProjectSync } = await import("./store/effects/zap-project-sync");
  installZapProjectSync();

  const detached = getDetachedParams();

  if (detached) {
    const { DetachedView } = await import("./components/DetachedView/DetachedView");
    ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
      <React.StrictMode>
        <I18nextProvider i18n={i18n}>
          <ErrorBoundary>
            <DetachedView sessionId={detached.sessionId} tabType={detached.tabType} />
          </ErrorBoundary>
        </I18nextProvider>
      </React.StrictMode>
    );
    return;
  }

  const { default: App } = await import("./App");
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <I18nextProvider i18n={i18n}>
        <ErrorBoundary>
          <App />
        </ErrorBoundary>
      </I18nextProvider>
    </React.StrictMode>
  );
}

initApp();

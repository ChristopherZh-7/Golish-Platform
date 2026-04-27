import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type React from "react";
import { useEffect } from "react";
import { getProjectPath } from "@/lib/projects";
import { logger } from "@/lib/logger";
import { notify } from "@/lib/notify";
import { useStore } from "../../store";

interface UseTabSplitEventsProps {
  setRightPanelTabs: React.Dispatch<React.SetStateAction<string[]>>;
  setRightActiveTab: React.Dispatch<React.SetStateAction<string | null>>;
  setShowSplitDropZone: React.Dispatch<React.SetStateAction<boolean>>;
}

/**
 * Handles tab split, detach, tool output detection, and recording events.
 * Extracted from useAppLifecycle to reduce its responsibility scope.
 */
export function useTabSplitEvents({
  setRightPanelTabs,
  setRightActiveTab,
  setShowSplitDropZone,
}: UseTabSplitEventsProps) {
  // biome-ignore lint/correctness/useExhaustiveDependencies: preserve original empty deps; setters are stable
  useEffect(() => {
    const handleSplitTab = (e: Event) => {
      const tabId = (e as CustomEvent<string>).detail;
      setRightPanelTabs((prev) => {
        if (prev.includes(tabId)) return prev;
        const s = useStore.getState();
        if (s.activeSessionId === tabId) {
          const other = s.tabOrder.find(
            (id) =>
              id !== tabId &&
              !prev.includes(id) &&
              (s.sessions[id]?.tabType ?? "terminal") !== "home",
          );
          if (other) s.setActiveSession(other);
        }
        return [...prev, tabId];
      });
      setRightActiveTab(tabId);
      setShowSplitDropZone(false);
    };

    const handleUnsplitTab = () => {
      setRightPanelTabs([]);
      setRightActiveTab(null);
    };

    const handleDragHint = (e: Event) =>
      setShowSplitDropZone((e as CustomEvent<boolean>).detail);

    const handleToolOutput = async (e: Event) => {
      if (localStorage.getItem("golish-auto-detect-output") === "false") return;
      const { command, output } = (
        e as CustomEvent<{ command: string; output: string; sessionId: string }>
      ).detail;
      try {
        const detected = await invoke<{
          tool_id: string;
          tool_name: string;
          output_config: {
            format: string;
            produces: string[];
            patterns: unknown[];
            fields: Record<string, string>;
            detect?: string;
          };
        } | null>("output_detect_tool", { command, rawOutput: output });
        if (!detected) return;
        const parsed = await invoke<{
          items: { data_type: string; fields: Record<string, string> }[];
        }>("output_parse", {
          rawOutput: output,
          config: detected.output_config,
          toolId: detected.tool_id,
          toolName: detected.tool_name,
        });
        if (!parsed.items.length) return;
        const pp = getProjectPath();
        const produces = detected.output_config.produces;

        if (produces.includes("vulnerability")) {
          const vulnItems = parsed.items
            .filter((it) => it.data_type === "vulnerability")
            .map((it) => it.fields);
          if (vulnItems.length > 0) {
            const added = await invoke<number>("findings_import_parsed", {
              items: vulnItems,
              toolName: detected.tool_name,
              projectPath: pp,
            });
            if (added > 0) {
              notify.success(`${detected.tool_name}: ${added} findings imported`);
            }
          }
        }
      } catch {
        /* ignore */
      }
    };

    const handleDetachTab = async (e: Event) => {
      const { tabId, screenX, screenY } = (
        e as CustomEvent<{ tabId: string; screenX: number; screenY: number }>
      ).detail;
      const s = useStore.getState();
      const session = s.sessions[tabId];
      if (!session) return;
      const tabType = session.tabType ?? "terminal";

      if (tabType === "security") {
        const pseudoId = `security-all-${Date.now()}`;
        try {
          await invoke("create_detached_window", {
            sessionId: pseudoId,
            tabType: "security-all",
            title: "Security — Detached",
            x: screenX - 50,
            y: screenY - 20,
            width: 1000.0,
            height: 700.0,
          });
          notify.info("Security detached to floating window");
        } catch (err) {
          logger.error("[App] detach security tab failed:", err);
        }
        return;
      }

      if (tabType !== "terminal") return;

      const title =
        session.customName ||
        session.processName ||
        session.workingDirectory?.split(/[/\\]/).pop() ||
        "Terminal";

      try {
        await invoke("create_detached_window", {
          sessionId: tabId,
          tabType,
          title: `${title} — Detached`,
          x: screenX - 50,
          y: screenY - 20,
          width: 800.0,
          height: 500.0,
        });
        const detached = JSON.parse(localStorage.getItem("golish-detached-tabs") || "{}");
        detached[tabId] = { title, tabType };
        try {
          localStorage.setItem("golish-detached-tabs", JSON.stringify(detached));
        } catch {
          /* ignore */
        }

        const other = s.tabOrder.find(
          (id) => id !== tabId && (s.sessions[id]?.tabType ?? "terminal") !== "home",
        );
        if (other) s.setActiveSession(other);
        notify.info(`"${title}" detached to floating window`);
      } catch (err) {
        logger.error("[App] detach tab failed:", err);
      }
    };

    const handleDetachSecurityTab = async (e: Event) => {
      const { tabId, screenX, screenY } = (
        e as CustomEvent<{ tabId: string; screenX: number; screenY: number }>
      ).detail;
      const tabLabels: Record<string, string> = {
        history: "HTTP History",
        sitemap: "Site Map",
        scanner: "Scanner",
        repeater: "Repeater",
        alerts: "Alerts",
        audit: "Audit Log",
        passive: "Passive Scan",
        vault: "Credential Vault",
      };
      const title = tabLabels[tabId] || tabId;
      const pseudoId = `security-${tabId}-${Date.now()}`;

      try {
        await invoke("create_detached_window", {
          sessionId: pseudoId,
          tabType: `security-${tabId}`,
          title: `${title} — Detached`,
          x: screenX - 50,
          y: screenY - 20,
          width: 900.0,
          height: 600.0,
        });
        notify.info(`"${title}" detached to floating window`);
      } catch (err) {
        logger.error("[App] detach security tab failed:", err);
      }
    };

    const handleRecordingSaved = () => {
      notify.success("Terminal recording saved");
    };

    window.addEventListener("split-tab-right", handleSplitTab);
    window.addEventListener("unsplit-tab", handleUnsplitTab);
    window.addEventListener("tab-drag-split-hint", handleDragHint);
    window.addEventListener("detach-tab", handleDetachTab);
    window.addEventListener("detach-security-tab", handleDetachSecurityTab);
    window.addEventListener("tool-output-completed", handleToolOutput);
    window.addEventListener("recording-saved", handleRecordingSaved);

    let unlistenDetachedClose: (() => void) | null = null;
    listen<{ session_id: string }>("detached-window-closed", (event) => {
      const { session_id } = event.payload;
      const detached = JSON.parse(localStorage.getItem("golish-detached-tabs") || "{}");
      delete detached[session_id];
      try {
        localStorage.setItem("golish-detached-tabs", JSON.stringify(detached));
      } catch {
        /* ignore */
      }
      notify.info("Detached window closed — tab restored");
    }).then((fn) => {
      unlistenDetachedClose = fn;
    });

    return () => {
      window.removeEventListener("split-tab-right", handleSplitTab);
      window.removeEventListener("unsplit-tab", handleUnsplitTab);
      window.removeEventListener("tab-drag-split-hint", handleDragHint);
      window.removeEventListener("detach-tab", handleDetachTab);
      window.removeEventListener("detach-security-tab", handleDetachSecurityTab);
      window.removeEventListener("tool-output-completed", handleToolOutput);
      window.removeEventListener("recording-saved", handleRecordingSaved);
      unlistenDetachedClose?.();
    };
  }, []);
}

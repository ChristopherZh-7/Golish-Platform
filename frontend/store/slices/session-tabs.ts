/**
 * Session tab/pane management actions: opening special tabs, closing tabs,
 * reordering, tab activity, and moving tabs into panes.
 */

import { logger } from "@/lib/logger";
import { countLeafPanes, getAllLeafPanes } from "@/lib/pane-utils";
import { TerminalInstanceManager } from "@/lib/terminal/TerminalInstanceManager";
import {
  markTabNewActivityInDraft,
  purgeSessionStateInDraft,
} from "./session-helpers";
import type { ImmerSet, StateGet } from "./types";

export function createSessionTabActions(set: ImmerSet<any>, get: StateGet<any>) {
  return {
    openSettingsTab: () =>
      set((state: any) => {
        const existingSettingsTab = Object.values<any>(state.sessions).find(
          (session) => session.tabType === "settings",
        );

        if (existingSettingsTab) {
          state.activeSessionId = existingSettingsTab.id;
          state.tabHasNewActivity[existingSettingsTab.id] = false;
          const histIdx = state.tabActivationHistory.indexOf(existingSettingsTab.id);
          if (histIdx !== -1) {
            state.tabActivationHistory.splice(histIdx, 1);
          }
          state.tabActivationHistory.push(existingSettingsTab.id);
          return;
        }

        const settingsId = `settings-${Date.now()}`;
        state.sessions[settingsId] = {
          id: settingsId,
          tabType: "settings",
          name: "Settings",
          workingDirectory: "",
          createdAt: new Date().toISOString(),
          mode: "terminal",
        };
        state.activeSessionId = settingsId;
        state.tabLayouts = state.tabLayouts ?? {};
        state.tabLayouts[settingsId] = {
          root: { type: "leaf", id: settingsId, sessionId: settingsId },
          focusedPaneId: settingsId,
        };
        state.tabHasNewActivity[settingsId] = false;
        state.tabOrder.push(settingsId);
        state.tabActivationHistory.push(settingsId);
      }),

    openHomeTab: () =>
      set((state: any) => {
        const existingHomeTab = Object.values<any>(state.sessions).find(
          (session) => session.tabType === "home",
        );

        if (existingHomeTab) {
          state.activeSessionId = existingHomeTab.id;
          state.tabHasNewActivity[existingHomeTab.id] = false;
          const histIdx = state.tabActivationHistory.indexOf(existingHomeTab.id);
          if (histIdx !== -1) {
            state.tabActivationHistory.splice(histIdx, 1);
          }
          state.tabActivationHistory.push(existingHomeTab.id);
          return;
        }

        const homeId = `home-${Date.now()}`;
        state.sessions[homeId] = {
          id: homeId,
          tabType: "home",
          name: "Home",
          workingDirectory: "",
          createdAt: new Date().toISOString(),
          mode: "terminal",
        };
        state.activeSessionId = homeId;
        state.homeTabId = homeId;
        state.tabLayouts = state.tabLayouts ?? {};
        state.tabLayouts[homeId] = {
          root: { type: "leaf", id: homeId, sessionId: homeId },
          focusedPaneId: homeId,
        };
        state.tabHasNewActivity[homeId] = false;
        state.tabOrder.unshift(homeId);
        state.tabActivationHistory.push(homeId);
      }),

    openBrowserTab: (url?: string) =>
      set((state: any) => {
        const existingBrowserTab = Object.values<any>(state.sessions).find(
          (session) => session.tabType === "browser",
        );

        if (existingBrowserTab) {
          state.activeSessionId = existingBrowserTab.id;
          state.tabHasNewActivity[existingBrowserTab.id] = false;
          const histIdx = state.tabActivationHistory.indexOf(existingBrowserTab.id);
          if (histIdx !== -1) {
            state.tabActivationHistory.splice(histIdx, 1);
          }
          state.tabActivationHistory.push(existingBrowserTab.id);
          return;
        }

        const browserId = `browser-${Date.now()}`;
        state.sessions[browserId] = {
          id: browserId,
          tabType: "browser",
          name: "Browser",
          workingDirectory: url || "",
          createdAt: new Date().toISOString(),
          mode: "terminal",
        };
        state.activeSessionId = browserId;
        state.tabLayouts = state.tabLayouts ?? {};
        state.tabLayouts[browserId] = {
          root: { type: "leaf", id: browserId, sessionId: browserId },
          focusedPaneId: browserId,
        };
        state.tabHasNewActivity[browserId] = false;
        state.tabOrder.push(browserId);
        state.tabActivationHistory.push(browserId);
      }),

    openSecurityTab: () =>
      set((state: any) => {
        const existingTab = Object.values<any>(state.sessions).find(
          (session) => session.tabType === "security",
        );

        if (existingTab) {
          state.activeSessionId = existingTab.id;
          state.tabHasNewActivity[existingTab.id] = false;
          const histIdx = state.tabActivationHistory.indexOf(existingTab.id);
          if (histIdx !== -1) {
            state.tabActivationHistory.splice(histIdx, 1);
          }
          state.tabActivationHistory.push(existingTab.id);
          return;
        }

        const securityId = `security-${Date.now()}`;
        state.sessions[securityId] = {
          id: securityId,
          tabType: "security",
          name: "Security",
          workingDirectory: "",
          createdAt: new Date().toISOString(),
          mode: "terminal",
        };
        state.activeSessionId = securityId;
        state.tabLayouts = state.tabLayouts ?? {};
        state.tabLayouts[securityId] = {
          root: { type: "leaf", id: securityId, sessionId: securityId },
          focusedPaneId: securityId,
        };
        state.tabHasNewActivity[securityId] = false;
        state.tabOrder.push(securityId);
        state.tabActivationHistory.push(securityId);
      }),

    getTabSessionIds: (tabId: string) => {
      const layout = (get() as any).tabLayouts?.[tabId];
      if (!layout) return [];
      return getAllLeafPanes(layout.root).map((pane) => pane.sessionId);
    },

    closeTab: (tabId: string) => {
      const currentState = get() as any;
      const layout = currentState.tabLayouts?.[tabId];
      const sessionIdsToClean: string[] = [];

      if (!layout) {
        sessionIdsToClean.push(tabId);
      } else {
        const panes = getAllLeafPanes(layout.root);
        for (const pane of panes) {
          sessionIdsToClean.push(pane.sessionId);
        }
      }

      for (const sessionId of sessionIdsToClean) {
        TerminalInstanceManager.dispose(sessionId);
      }

      import("@/hooks/useAiEvents").then(({ resetSessionSequence }) => {
        for (const sessionId of sessionIdsToClean) {
          resetSessionSequence(sessionId);
        }
      });

      set((state: any) => {
        const layout = state.tabLayouts?.[tabId];
        if (!layout) {
          purgeSessionStateInDraft(state, tabId);

          state.tabActivationHistory = state.tabActivationHistory.filter(
            (id: string) => id !== tabId,
          );
          if (state.activeSessionId === tabId) {
            state.activeSessionId =
              state.tabActivationHistory[state.tabActivationHistory.length - 1] ?? null;
          }
          return;
        }

        const panes = getAllLeafPanes(layout.root);
        for (const pane of panes) {
          purgeSessionStateInDraft(state, pane.sessionId);
        }

        delete state.tabLayouts[tabId];
        delete state.tabHasNewActivity[tabId];
        const tabOrderIdx = state.tabOrder.indexOf(tabId);
        if (tabOrderIdx !== -1) {
          state.tabOrder.splice(tabOrderIdx, 1);
        }

        state.tabActivationHistory = state.tabActivationHistory.filter(
          (id: string) => id !== tabId,
        );
        if (state.activeSessionId === tabId) {
          state.activeSessionId =
            state.tabActivationHistory[state.tabActivationHistory.length - 1] ?? null;
        }
      });
    },

    markTabNewActivityBySession: (sessionId: string) =>
      set((state: any) => {
        markTabNewActivityInDraft(state, sessionId);
      }),

    clearTabNewActivity: (tabId: string) =>
      set((state: any) => {
        state.tabHasNewActivity[tabId] = false;
      }),

    moveTab: (tabId: string, direction: "left" | "right") =>
      set((state: any) => {
        const idx = state.tabOrder.indexOf(tabId);
        if (idx === -1) return;
        if (idx === 0) return;
        const targetIdx = direction === "left" ? idx - 1 : idx + 1;
        if (targetIdx < 1 || targetIdx >= state.tabOrder.length) return;
        const temp = state.tabOrder[targetIdx];
        state.tabOrder[targetIdx] = state.tabOrder[idx];
        state.tabOrder[idx] = temp;
      }),

    reorderTab: (draggedTabId: string, targetTabId: string) =>
      set((state: any) => {
        if (draggedTabId === targetTabId) return;
        const fromIdx = state.tabOrder.indexOf(draggedTabId);
        const toIdx = state.tabOrder.indexOf(targetTabId);
        if (fromIdx < 1 || toIdx < 1) return;
        state.tabOrder.splice(fromIdx, 1);
        state.tabOrder.splice(toIdx, 0, draggedTabId);
      }),

    moveTabToPane: (
      sourceTabId: string,
      destTabId: string,
      location: "left" | "right" | "top" | "bottom",
    ) =>
      set((state: any) => {
        logger.info("[store] moveTabToPane: start", {
          sourceTabId,
          destTabId,
          location,
        });
        const sourceLayout = state.tabLayouts?.[sourceTabId];
        const destLayout = state.tabLayouts?.[destTabId];
        if (!sourceLayout || !destLayout) {
          logger.warn("[store] moveTabToPane: missing layout", {
            hasSourceLayout: !!sourceLayout,
            hasDestLayout: !!destLayout,
          });
          return;
        }
        const sourceSession = state.sessions[sourceTabId];
        if (!sourceSession) {
          logger.warn("[store] moveTabToPane: source session missing", { sourceTabId });
          return;
        }
        const sourceTabType = sourceSession.tabType ?? "terminal";
        if (sourceTabType !== "terminal") {
          logger.warn("[store] moveTabToPane: source not terminal", {
            sourceTabId,
            sourceTabType,
          });
          return;
        }
        const destSession = state.sessions[destTabId];
        if (!destSession) {
          logger.warn("[store] moveTabToPane: destination session missing", { destTabId });
          return;
        }
        const destTabType = destSession.tabType ?? "terminal";
        if (destTabType !== "terminal") {
          logger.warn("[store] moveTabToPane: destination not terminal", {
            destTabId,
            destTabType,
          });
          return;
        }
        const destPaneCount = countLeafPanes(destLayout.root);
        const sourcePaneCount = countLeafPanes(sourceLayout.root);
        if (destPaneCount + sourcePaneCount > 4) {
          logger.warn("[store] moveTabToPane: pane limit exceeded", {
            destPaneCount,
            sourcePaneCount,
          });
          return;
        }

        const direction =
          location === "left" || location === "right" ? "vertical" : "horizontal";
        const newPaneId = crypto.randomUUID();

        if (location === "right" || location === "bottom") {
          state.tabLayouts[destTabId].root = {
            type: "split",
            id: crypto.randomUUID(),
            direction,
            children: [
              destLayout.root,
              { type: "leaf", id: newPaneId, sessionId: sourceTabId },
            ],
            ratio: 0.5,
          };
        } else {
          state.tabLayouts[destTabId].root = {
            type: "split",
            id: crypto.randomUUID(),
            direction,
            children: [
              { type: "leaf", id: newPaneId, sessionId: sourceTabId },
              destLayout.root,
            ],
            ratio: 0.5,
          };
        }

        delete state.tabLayouts[sourceTabId];

        const tabOrderIdx = state.tabOrder.indexOf(sourceTabId);
        if (tabOrderIdx !== -1) {
          state.tabOrder.splice(tabOrderIdx, 1);
        }

        delete state.tabHasNewActivity[sourceTabId];

        if (state.activeSessionId === sourceTabId) {
          state.activeSessionId = destTabId;
          const histIdx = state.tabActivationHistory.indexOf(destTabId);
          if (histIdx !== -1) {
            state.tabActivationHistory.splice(histIdx, 1);
          }
          state.tabActivationHistory.push(destTabId);
        }

        state.tabLayouts[destTabId].focusedPaneId = newPaneId;
        logger.info("[store] moveTabToPane: completed", {
          sourceTabId,
          destTabId,
          newPaneId,
          direction,
        });
      }),
  };
}

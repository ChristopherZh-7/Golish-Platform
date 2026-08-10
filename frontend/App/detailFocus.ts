import type { DetailViewMode } from "@/store/types/session";

/**
 * Detail views temporarily own the main workspace so dense Agent/evidence
 * content is not squeezed by the ChatPanel sidebar.
 */
export function isDetailFocusMode(mode: DetailViewMode | undefined): boolean {
  return mode === "tool-detail" || mode === "sub-agent-detail";
}

/** Keep the ChatPanel component alive for its conversation event projection. */
export function shouldMountAiChatPanel(isOnHomeTab: boolean): boolean {
  return !isOnHomeTab;
}

export function shouldHideAiChatPanel(
  chatPanelVisible: boolean,
  mode: DetailViewMode | undefined
): boolean {
  return !chatPanelVisible || isDetailFocusMode(mode);
}

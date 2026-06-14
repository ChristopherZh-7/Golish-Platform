/**
 * Composed Zustand store. Composes slices + re-exports `./public-api.ts`.
 * Root-level fields live in `./slices/app-shell.ts`.
 */

import { enableMapSet } from "immer";
import { create } from "zustand";
import { devtools } from "zustand/middleware";
import { immer } from "zustand/middleware/immer";
import * as S from "./slices";

enableMapSet();

interface GolishState
  extends S.AppearanceSlice,
    S.ContextSlice,
    S.ConversationSlice,
    S.DialogSlice,
    S.NotificationSlice,
    S.PanelSlice,
    S.SessionSlice,
    S.AiSlice,
    S.WorkflowSlice,
    S.PaneSlice,
    S.HitlSlice,
    S.AppShellSlice {}

export const useStore = create<GolishState>()(
  devtools(
    immer((set, get, _store) => ({
      ...S.createAppearanceSlice(set, get),
      ...S.createContextSlice(set, get),
      ...S.createConversationSlice(set, get),
      ...S.createDialogSlice(set, get),
      ...S.createNotificationSlice(set, get),
      ...S.createPanelSlice(set, get),
      ...S.createSessionSlice(set, get),
      ...S.createAiSlice(set, get),
      ...S.createWorkflowSlice(set, get),
      ...S.createPaneSlice(set, get),
      ...S.createHitlSlice(set, get),
      ...S.createAppShellSlice(set, get),
    })),
    { name: "golish" }
  )
);

export * from "./public-api";

import { installDevTools } from "./dev-mock";

installDevTools();

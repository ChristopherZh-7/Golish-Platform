import { useEffect, useRef } from "react";
import { onEvent } from "@/lib/events";
import { logger } from "@/lib/logger";
import type { SidecarEventType } from "@/lib/sidecar";

/**
 * Hook to subscribe to sidecar events from the Tauri backend.
 *
 * Events are emitted on the "sidecar-event" channel and include:
 * - Session lifecycle: session_started, session_ended
 * - Patch operations: patch_created, patch_applied, patch_discarded, patch_message_updated
 * - Artifact operations: artifact_created, artifact_applied, artifact_discarded
 *
 * @param handler Callback invoked for each sidecar event
 *
 * @example
 * ```tsx
 * useSidecarEvents((event) => {
 *   switch (event.event_type) {
 *     case "patch_created":
 *       console.log(`New patch: ${event.subject}`);
 *       break;
 *     case "artifact_applied":
 *       console.log(`Applied: ${event.filename} to ${event.target}`);
 *       break;
 *   }
 * });
 * ```
 */
export function useSidecarEvents(handler: (event: SidecarEventType) => void): void {
  const callbackRef = useRef(handler);

  useEffect(() => {
    callbackRef.current = handler;
  }, [handler]);

  useEffect(() => {
    let isMounted = true;
    let unlisten: (() => void) | null = null;

    const setupListener = async () => {
      try {
        unlisten = await onEvent("sidecar-event", (payload) => {
          if (isMounted) {
            callbackRef.current(payload);
          }
        });
      } catch (error) {
        logger.error("Failed to subscribe to sidecar events:", error);
      }
    };

    setupListener();

    return () => {
      isMounted = false;
      if (unlisten) {
        unlisten();
      }
    };
  }, []);
}

/**
 * Type guards for sidecar events
 */
export const SidecarEventGuards = {
  isSessionStarted(
    event: SidecarEventType
  ): event is Extract<SidecarEventType, { event_type: "session_started" }> {
    return event.event_type === "session_started";
  },

  isSessionEnded(
    event: SidecarEventType
  ): event is Extract<SidecarEventType, { event_type: "session_ended" }> {
    return event.event_type === "session_ended";
  },

  isPatchCreated(
    event: SidecarEventType
  ): event is Extract<SidecarEventType, { event_type: "patch_created" }> {
    return event.event_type === "patch_created";
  },

  isPatchApplied(
    event: SidecarEventType
  ): event is Extract<SidecarEventType, { event_type: "patch_applied" }> {
    return event.event_type === "patch_applied";
  },

  isPatchDiscarded(
    event: SidecarEventType
  ): event is Extract<SidecarEventType, { event_type: "patch_discarded" }> {
    return event.event_type === "patch_discarded";
  },

  isPatchMessageUpdated(
    event: SidecarEventType
  ): event is Extract<SidecarEventType, { event_type: "patch_message_updated" }> {
    return event.event_type === "patch_message_updated";
  },

  isArtifactCreated(
    event: SidecarEventType
  ): event is Extract<SidecarEventType, { event_type: "artifact_created" }> {
    return event.event_type === "artifact_created";
  },

  isArtifactApplied(
    event: SidecarEventType
  ): event is Extract<SidecarEventType, { event_type: "artifact_applied" }> {
    return event.event_type === "artifact_applied";
  },

  isArtifactDiscarded(
    event: SidecarEventType
  ): event is Extract<SidecarEventType, { event_type: "artifact_discarded" }> {
    return event.event_type === "artifact_discarded";
  },

  /** Check if event is any patch-related event */
  isPatchEvent(event: SidecarEventType): boolean {
    return event.event_type.startsWith("patch_");
  },

  /** Check if event is any artifact-related event */
  isArtifactEvent(event: SidecarEventType): boolean {
    return event.event_type.startsWith("artifact_");
  },

  /** Check if event is any session-related event */
  isSessionEvent(event: SidecarEventType): boolean {
    return event.event_type.startsWith("session_");
  },
};

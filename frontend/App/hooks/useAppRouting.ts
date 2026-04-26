import { useEffect, useState } from "react";
import type { ActivityView } from "../../components/ActivityBar/ActivityBar";
import type { PageRoute } from "../../components/CommandPalette";

/**
 * Owns the local routing state used by App: the current `PageRoute` (main vs
 * testbed), the current ActivityBar view, and the set of activity views the
 * user has visited so we can keep their DOM mounted for cheap re-shows.
 *
 * Also wires the `open-activity-view` / `close-activity-view` window events
 * that child panels (e.g. TargetPanel) dispatch to control the overlay.
 */
export function useAppRouting() {
  const [currentPage, setCurrentPage] = useState<PageRoute>("main");
  const [activityView, setActivityView] = useState<ActivityView>(null);
  const [visitedViews, setVisitedViews] = useState<Set<string>>(new Set());

  useEffect(() => {
    if (activityView && !visitedViews.has(activityView)) {
      setVisitedViews((prev) => new Set(prev).add(activityView));
    }
  }, [activityView, visitedViews]);

  // Allow child components (e.g. TargetPanel) to close the activity overlay
  useEffect(() => {
    const closeHandler = () => setActivityView(null);
    const openHandler = (e: Event) => {
      const view = (e as CustomEvent<ActivityView>).detail;
      if (view) setActivityView(view);
    };
    window.addEventListener("close-activity-view", closeHandler);
    window.addEventListener("open-activity-view", openHandler);
    return () => {
      window.removeEventListener("close-activity-view", closeHandler);
      window.removeEventListener("open-activity-view", openHandler);
    };
  }, []);

  return {
    currentPage,
    setCurrentPage,
    activityView,
    setActivityView,
    visitedViews,
  };
}

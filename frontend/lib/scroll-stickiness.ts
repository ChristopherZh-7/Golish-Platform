export const DEFAULT_STICKY_BOTTOM_THRESHOLD_PX = 96;
const SCROLL_UP_EPSILON_PX = 1;

export interface ScrollMetrics {
  scrollTop: number;
  scrollHeight: number;
  clientHeight: number;
}

export function isNearScrollBottom(
  metrics: ScrollMetrics,
  thresholdPx = DEFAULT_STICKY_BOTTOM_THRESHOLD_PX
): boolean {
  return metrics.scrollHeight - metrics.scrollTop - metrics.clientHeight <= thresholdPx;
}

export function shouldStickToBottomAfterScroll(
  previousScrollTop: number,
  metrics: ScrollMetrics,
  thresholdPx = DEFAULT_STICKY_BOTTOM_THRESHOLD_PX
): boolean {
  if (metrics.scrollTop < previousScrollTop - SCROLL_UP_EPSILON_PX) {
    return false;
  }
  return isNearScrollBottom(metrics, thresholdPx);
}

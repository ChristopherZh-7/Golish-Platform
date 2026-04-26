/**
 * Determine whether two step lists represent a structural change (different
 * steps, not just status updates). Uses step IDs when available; for ID-less
 * steps, normalises the text and compares Jaccard similarity >= 0.5.
 */
export function planStepsStructurallyChanged(
  prev: Array<{ id?: string; step: string }>,
  next: Array<{ id?: string; step: string }>,
): boolean {
  if (prev.length !== next.length) return true;

  const allHaveIds = prev.every((s) => s.id) && next.every((s) => s.id);
  if (allHaveIds) {
    const prevIds = new Set(prev.map((s) => s.id));
    return next.some((s) => !prevIds.has(s.id));
  }

  for (let i = 0; i < prev.length; i++) {
    const pId = prev[i].id;
    const nId = next[i].id;
    if (pId && nId) {
      if (pId !== nId) return true;
      continue;
    }
    const pWords = new Set(
      prev[i].step.toLowerCase().replace(/[^\w\s]/g, "").split(/\s+/).filter(Boolean),
    );
    const nWords = new Set(
      next[i].step.toLowerCase().replace(/[^\w\s]/g, "").split(/\s+/).filter(Boolean),
    );
    const union = new Set([...pWords, ...nWords]);
    const intersection = [...pWords].filter((w) => nWords.has(w)).length;
    if (union.size > 0 && intersection / union.size < 0.5) return true;
  }
  return false;
}

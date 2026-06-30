import type { ApiEndpoint } from "@/lib/security-analysis";

function paramName(param: unknown): string | null {
  if (typeof param === "string") return param.trim() || null;
  if (typeof param === "number" || typeof param === "boolean") return String(param);
  if (!param || typeof param !== "object") return null;

  const record = param as Record<string, unknown>;
  for (const key of ["name", "key", "param", "parameter"]) {
    const value = record[key];
    if (typeof value === "string" && value.trim()) return value.trim();
  }
  return null;
}

export function getEndpointParamNames(params: unknown): string[] {
  if (!Array.isArray(params)) return [];
  return [...new Set(params.map(paramName).filter(Boolean) as string[])].sort((a, b) =>
    a.localeCompare(b)
  );
}

export function countEndpointParams(endpoints: ApiEndpoint[]): number {
  return endpoints.reduce(
    (sum, endpoint) => sum + getEndpointParamNames(endpoint.params).length,
    0
  );
}

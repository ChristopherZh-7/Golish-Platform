export type BodyMode = "json" | "text";

export function bodyRenderMode(contentType: string): BodyMode {
  return /json/i.test(contentType) ? "json" : "text";
}

export function prettyBody(mode: BodyMode, body: string): string {
  if (mode !== "json") return body;
  try {
    return JSON.stringify(JSON.parse(body), null, 2);
  } catch {
    return body;
  }
}

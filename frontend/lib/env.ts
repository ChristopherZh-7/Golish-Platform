export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export type Platform = "macos" | "windows" | "linux";

let cachedPlatform: Platform | null = null;

export function getPlatform(): Platform {
  if (cachedPlatform) return cachedPlatform;
  const ua = navigator.userAgent.toLowerCase();
  if (ua.includes("win")) cachedPlatform = "windows";
  else if (ua.includes("mac")) cachedPlatform = "macos";
  else cachedPlatform = "linux";
  return cachedPlatform;
}

export function isWindows(): boolean {
  return getPlatform() === "windows";
}

export function isMac(): boolean {
  return getPlatform() === "macos";
}

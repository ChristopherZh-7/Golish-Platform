const MULTI_PART_TLDS = new Set([
  "co.uk", "co.jp", "co.kr", "co.nz", "co.za", "co.in", "co.id",
  "com.au", "com.br", "com.cn", "com.hk", "com.mx", "com.sg", "com.tw",
  "net.au", "net.cn", "org.au", "org.uk", "org.cn",
  "ac.uk", "gov.uk", "gov.cn", "edu.cn", "edu.au",
]);

/**
 * Extract the registrable root domain from a URL or hostname.
 * Handles multi-part TLDs (e.g. co.uk), IPv4, and IPv6.
 *
 * @param value - A URL string or bare hostname
 * @param mode  - "full" checks multi-part TLDs; "simple" just takes the last two labels
 */
export function getRootDomain(value: string, mode: "full" | "simple" = "full"): string {
  let host: string;
  try {
    const u = new URL(value.includes("://") ? value : `https://${value}`);
    host = u.hostname;
  } catch {
    host = value.replace(/\/.*$/, "");
  }

  const bare = host.replace(/:\d+$/, "");
  if (/^\d{1,3}(\.\d{1,3}){3}$/.test(bare)) return host;
  if (bare.startsWith("[") || bare.includes(":")) return host;

  const parts = bare.split(".");
  if (parts.length <= 2) return host;

  if (mode === "full") {
    const last2 = parts.slice(-2).join(".");
    if (MULTI_PART_TLDS.has(last2) && parts.length > 2) {
      return parts.slice(-3).join(".");
    }
    return last2;
  }

  return parts.slice(-2).join(".");
}

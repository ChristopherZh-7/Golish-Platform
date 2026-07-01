import type { Target } from "@/lib/pentest/types";

const IPV4_RE = /^\d{1,3}(?:\.\d{1,3}){3}$/;
const UNRESOLVED_GROUP_ID = "__unresolved_assets__";
const DOMAIN_ALIAS_TARGET_TYPES = new Set(["domain", "subdomain", "host"]);

export interface TargetAssetGroup {
  id: string;
  label: string;
  host: string | null;
  ipTarget: Target | null;
  targets: Target[];
  linkedTargets: Target[];
  inScope: number;
  outScope: number;
}

function normalizeHost(value: string | null | undefined): string {
  return (value ?? "").trim().replace(/^\[|\]$/g, "");
}

function ipHostFromValue(value: string | null | undefined): string | null {
  const raw = normalizeHost(value);
  if (!raw) return null;
  if (IPV4_RE.test(raw)) return raw;
  try {
    const parsed = new URL(raw);
    const host = normalizeHost(parsed.hostname);
    return IPV4_RE.test(host) ? host : null;
  } catch {
    return null;
  }
}

export function isIpLiteralTargetValue(target: Target): boolean {
  return ipHostFromValue(target.value) != null;
}

function hostKeyForTarget(target: Target): string | null {
  if (target.type === "ip") return normalizeHost(target.value);
  const realIp = normalizeHost(target.real_ip);
  if (realIp) return realIp;
  return ipHostFromValue(target.value);
}

function sortTargets(a: Target, b: Target): number {
  if (a.type === "ip" && b.type !== "ip") return -1;
  if (a.type !== "ip" && b.type === "ip") return 1;
  return a.value.localeCompare(b.value, "zh");
}

function normalizedDomainAliasHost(target: Target): string | null {
  if (!DOMAIN_ALIAS_TARGET_TYPES.has(target.type)) return null;
  const raw = normalizeHost(target.value).toLowerCase().replace(/\.$/, "");
  if (!raw) return null;
  try {
    const parsed = new URL(raw);
    return normalizeHost(parsed.hostname).toLowerCase().replace(/\.$/, "") || null;
  } catch {
    return raw;
  }
}

function domainAliasDisplayKey(target: Target): string {
  const host = normalizedDomainAliasHost(target);
  if (!host) return `${target.type}:${target.value}`;
  return host.startsWith("www.") ? host.slice(4) : host;
}

function isWwwAliasTarget(target: Target): boolean {
  return normalizedDomainAliasHost(target)?.startsWith("www.") ?? false;
}

function preferLinkedAlias(candidate: Target, current: Target): Target {
  const candidateIsWww = isWwwAliasTarget(candidate);
  const currentIsWww = isWwwAliasTarget(current);
  if (candidateIsWww !== currentIsWww) return candidateIsWww ? current : candidate;
  if (candidate.scope !== current.scope) return candidate.scope === "in" ? candidate : current;
  return candidate.value.length < current.value.length ? candidate : current;
}

function dedupeDisplayLinkedTargets(targets: Target[]): Target[] {
  const selected = new Map<string, Target>();
  for (const target of targets) {
    const key = domainAliasDisplayKey(target);
    const current = selected.get(key);
    selected.set(key, current ? preferLinkedAlias(target, current) : target);
  }
  return targets.filter((target) => selected.get(domainAliasDisplayKey(target))?.id === target.id);
}

export function groupTargetsByHost(targets: Target[], unresolvedLabel: string): TargetAssetGroup[] {
  const groups = new Map<string, TargetAssetGroup>();

  for (const target of targets) {
    const host = hostKeyForTarget(target);
    const id = host ? `host:${host}` : UNRESOLVED_GROUP_ID;
    let group = groups.get(id);
    if (!group) {
      group = {
        id,
        label: host ?? unresolvedLabel,
        host,
        ipTarget: null,
        targets: [],
        linkedTargets: [],
        inScope: 0,
        outScope: 0,
      };
      groups.set(id, group);
    }

    group.targets.push(target);
    if (target.scope === "in") group.inScope += 1;
    else group.outScope += 1;
    if (!group.ipTarget && target.type === "ip" && host === normalizeHost(target.value)) {
      group.ipTarget = target;
    }
  }

  const result = [...groups.values()].map((group) => {
    const sorted = [...group.targets].sort(sortTargets);
    const linkedTargets = dedupeDisplayLinkedTargets(
      sorted.filter((target) => target.id !== group.ipTarget?.id && !isIpLiteralTargetValue(target))
    );
    return { ...group, targets: sorted, linkedTargets };
  });

  result.sort((a, b) => {
    if (!a.host && b.host) return 1;
    if (a.host && !b.host) return -1;
    return a.label.localeCompare(b.label, "zh", { numeric: true });
  });

  return result;
}

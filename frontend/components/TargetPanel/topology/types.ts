import type { Organization } from "@/lib/api/organizations";
import type { PortInfo, Target } from "@/lib/pentest/types";

export type TopologyMode = "ownership" | "surface" | "evidence";
export type TopologyNodeKind = "organization" | "target" | "service" | "evidence";
export type TopologyEdgeKind = "owns" | "contains" | "exposes" | "produced";

export interface TopologyVisibility {
  organization: boolean;
  target: boolean;
  service: boolean;
  evidence: boolean;
}

export interface TargetTopologySurfaceSummary {
  endpoints: number;
  params: number;
  paths: number;
  js: number;
}

export interface TopologyNode {
  id: string;
  kind: TopologyNodeKind;
  label: string;
  subtitle: string;
  column: number;
  x: number;
  y: number;
  width: number;
  height: number;
  scope?: "in" | "out";
  organization?: Organization;
  target?: Target;
  port?: PortInfo;
  metrics?: {
    targets?: number;
    inScopeTargets?: number;
    ports?: number;
    endpoints?: number;
    params?: number;
    paths?: number;
    js?: number;
    evidence?: number;
    findings?: number;
  };
}

export interface TopologyEdge {
  id: string;
  source: string;
  target: string;
  kind: TopologyEdgeKind;
  label: string;
}

export interface TopologyModel {
  nodes: TopologyNode[];
  edges: TopologyEdge[];
  stats: {
    organizations: number;
    targets: number;
    services: number;
    endpoints: number;
    params: number;
    paths: number;
    js: number;
    evidence: number;
  };
  bounds: {
    width: number;
    height: number;
  };
}

/**
 * Shared target-type glyphs for the Target panel.
 *
 * `TYPE_ICONS` maps a target's `type` to its lucide glyph. Shared by the tree
 * rows (`TargetTreeRow`) and the workspace overview list (`OrgWorkspacePanel`),
 * so it lives in its own module rather than being duplicated.
 */

import { Crosshair, Globe, Hash, Network } from "lucide-react";
import type { ReactNode } from "react";

export const TYPE_ICONS: Record<string, ReactNode> = {
  domain: <Globe className="w-3 h-3 text-blue-400" />,
  ip: <Hash className="w-3 h-3 text-green-400" />,
  cidr: <Network className="w-3 h-3 text-yellow-400" />,
  url: <Globe className="w-3 h-3 text-purple-400" />,
  wildcard: <Crosshair className="w-3 h-3 text-orange-400" />,
};

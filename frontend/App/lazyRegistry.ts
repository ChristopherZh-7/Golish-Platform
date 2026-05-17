import { lazy } from "react";

// Lazy loaded components - these are not needed on initial render
// and can be loaded on-demand to reduce initial bundle size

export const FileEditorSidebarPanel = lazy(() =>
  import("../components/FileEditorSidebar").then((m) => ({
    default: m.FileEditorSidebarPanel,
  }))
);

export const GitPanel = lazy(() =>
  import("../components/GitPanel").then((m) => ({ default: m.GitPanel }))
);

export const SessionBrowser = lazy(() =>
  import("../components/SessionBrowser/SessionBrowser").then((m) => ({
    default: m.SessionBrowser,
  }))
);

export const SettingsDialog = lazy(() =>
  import("../components/Settings").then((m) => ({ default: m.SettingsDialog }))
);

export const SettingsNav = lazy(() =>
  import("../components/Settings").then((m) => ({ default: m.SettingsNav }))
);

export const SettingsContent = lazy(() =>
  import("../components/Settings").then((m) => ({ default: m.SettingsContent }))
);

export const ToolManagerView = lazy(() =>
  import("../components/ToolManager/ToolManager").then((m) => ({
    default: m.ToolManager,
  }))
);

export const WikiPanelView = lazy(() =>
  import("../components/WikiPanel/WikiPanel").then((m) => ({
    default: m.WikiPanel,
  }))
);

export const TargetPanelView = lazy(() =>
  import("../components/TargetPanel/TargetPanel").then((m) => ({
    default: m.TargetPanel,
  }))
);

export const DashboardPanelView = lazy(() =>
  import("../components/DashboardPanel").then((m) => ({
    default: m.DashboardPanel,
  }))
);

export const FindingsPanelView = lazy(() =>
  import("../components/FindingsPanel/FindingsPanel").then((m) => ({
    default: m.FindingsPanel,
  }))
);

export const PipelinePanelView = lazy(() =>
  import("../components/PipelinePanel/PipelinePanel").then((m) => ({
    default: m.PipelinePanel,
  }))
);

export const AuditLogPanelView = lazy(() =>
  import("../components/AuditLogPanel").then((m) => ({
    default: m.AuditLogPanel,
  }))
);

export const WordlistPanelView = lazy(() =>
  import("../components/WordlistPanel/WordlistPanel").then((m) => ({
    default: m.WordlistPanel,
  }))
);

export const VulnIntelPanelView = lazy(() =>
  import("../components/VulnIntelPanel/VulnIntelPanel").then((m) => ({
    default: m.VulnIntelPanel,
  }))
);

// `RecordingsPanelView` was retired in D6.4b together with the xterm.js
// renderer it depended on. Future replay UI would need a GridReplayer
// component (TODO: see Phase C handoff).

export const ContextPanel = lazy(() =>
  import("../components/Sidecar/ContextPanel").then((m) => ({
    default: m.ContextPanel,
  }))
);

export const SidecarPanel = lazy(() =>
  import("../components/Sidecar/SidecarPanel").then((m) => ({
    default: m.SidecarPanel,
  }))
);

export const ComponentTestbed = lazy(() =>
  import("../pages/ComponentTestbed").then((m) => ({
    default: m.ComponentTestbed,
  }))
);

export const QuickOpenDialog = lazy(() =>
  import("../components/QuickOpenDialog").then((m) => ({
    default: m.QuickOpenDialog,
  }))
);

export const KeyboardShortcutsHelp = lazy(() =>
  import("../components/KeyboardShortcutsHelp/KeyboardShortcutsHelp").then((m) => ({
    default: m.KeyboardShortcutsHelp,
  }))
);

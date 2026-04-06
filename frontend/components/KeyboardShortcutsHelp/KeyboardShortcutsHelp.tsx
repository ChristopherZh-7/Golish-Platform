import { memo } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

interface ShortcutEntry {
  keys: string;
  description: string;
}

interface ShortcutGroup {
  title: string;
  shortcuts: ShortcutEntry[];
}

const SHORTCUT_GROUPS: ShortcutGroup[] = [
  {
    title: "General",
    shortcuts: [
      { keys: "⌘ K", description: "Command Palette" },
      { keys: "⌘ ,", description: "Settings" },
      { keys: "⌘ P", description: "Quick Open File" },
      { keys: "⌘ /", description: "Keyboard Shortcuts" },
    ],
  },
  {
    title: "Tabs & Sessions",
    shortcuts: [
      { keys: "⌘ T", description: "New Tab" },
      { keys: "⌘ W", description: "Close Pane" },
      { keys: "⌘ [1-9]", description: "Switch to Tab" },
      { keys: "Ctrl ]", description: "Next Tab" },
      { keys: "Ctrl [", description: "Previous Tab" },
      { keys: "⌘ I", description: "Toggle Input Mode" },
    ],
  },
  {
    title: "Panels",
    shortcuts: [
      { keys: "⌘ B", description: "Open Browser" },
      { keys: "⌘ J", description: "Toggle Terminal" },
      { keys: "⌘ L", description: "Focus AI Chat" },
      { keys: "⌘ ⇧ S", description: "Open Security" },
      { keys: "⌘ ⇧ M", description: "Tool Manager" },
      { keys: "⌘ ⇧ W", description: "Wiki / Knowledge Base" },
      { keys: "⌘ ⇧ G", description: "Git Panel" },
      { keys: "⌘ ⇧ E", description: "File Editor Panel" },
      { keys: "⌘ ⇧ C", description: "Context Panel" },
      { keys: "⌘ ⇧ P", description: "Sidecar Panel" },
      { keys: "⌘ ⇧ F", description: "Toggle Full Terminal" },
    ],
  },
  {
    title: "Panes",
    shortcuts: [
      { keys: "⌘ D", description: "Split Right" },
      { keys: "⌘ ⇧ D", description: "Split Down" },
      { keys: "⌘ ⌥ ↑↓←→", description: "Navigate Panes" },
    ],
  },
];

interface KeyboardShortcutsHelpProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

export const KeyboardShortcutsHelp = memo(function KeyboardShortcutsHelp({
  open,
  onOpenChange,
}: KeyboardShortcutsHelpProps) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[520px] max-h-[80vh] overflow-y-auto bg-card border-border">
        <DialogHeader>
          <DialogTitle className="text-base font-semibold">Keyboard Shortcuts</DialogTitle>
        </DialogHeader>
        <div className="space-y-5 mt-2">
          {SHORTCUT_GROUPS.map((group) => (
            <div key={group.title}>
              <h3 className="text-xs font-medium text-muted-foreground uppercase tracking-wider mb-2">
                {group.title}
              </h3>
              <div className="space-y-1">
                {group.shortcuts.map((shortcut) => (
                  <div
                    key={shortcut.keys}
                    className="flex items-center justify-between py-1.5 px-2 rounded hover:bg-muted/50"
                  >
                    <span className="text-sm text-foreground">{shortcut.description}</span>
                    <kbd className="inline-flex items-center gap-1 px-2 py-0.5 rounded bg-muted text-xs font-mono text-muted-foreground border border-border">
                      {shortcut.keys}
                    </kbd>
                  </div>
                ))}
              </div>
            </div>
          ))}
        </div>
      </DialogContent>
    </Dialog>
  );
});

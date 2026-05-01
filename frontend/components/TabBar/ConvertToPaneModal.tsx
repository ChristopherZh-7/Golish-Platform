import React from "react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { TabItemState } from "@/store/selectors/tab-bar";

interface ConvertToPaneModalProps {
  sourceTabId: string;
  tabs: TabItemState[];
  onClose: () => void;
  onConfirm: (destTabId: string, location: "left" | "right" | "top" | "bottom") => void;
}

export function ConvertToPaneModal({
  sourceTabId,
  tabs,
  onClose,
  onConfirm,
}: ConvertToPaneModalProps) {
  const destTabs = tabs
    .map((t, index) => ({ tab: t, index }))
    .filter(({ tab }) => tab.tabType === "terminal" && tab.id !== sourceTabId);
  const [destTabId, setDestTabId] = React.useState(destTabs[0]?.tab.id ?? "");
  const [location, setLocation] = React.useState<"left" | "right" | "top" | "bottom">("right");

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-[400px]" onMouseDown={(e) => e.stopPropagation()}>
        <DialogHeader>
          <DialogTitle>Convert to Pane</DialogTitle>
          <DialogDescription>Move this tab as a pane into another tab.</DialogDescription>
        </DialogHeader>
        <div className="grid gap-4 py-2">
          <div className="grid gap-2">
            <span className="text-sm font-medium">Destination Tab</span>
            <Select value={destTabId} onValueChange={setDestTabId}>
              <SelectTrigger className="w-full">
                <SelectValue placeholder="Select a tab" />
              </SelectTrigger>
              <SelectContent>
                {destTabs.map(({ tab, index }) => (
                  <SelectItem key={tab.id} value={tab.id}>
                    <span className="text-muted-foreground mr-1.5">{index}.</span>
                    {tab.customName || tab.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="grid gap-2">
            <span className="text-sm font-medium">Placement</span>
            <Select value={location} onValueChange={(v) => setLocation(v as typeof location)}>
              <SelectTrigger className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="right">Right</SelectItem>
                <SelectItem value="left">Left</SelectItem>
                <SelectItem value="bottom">Bottom</SelectItem>
                <SelectItem value="top">Top</SelectItem>
              </SelectContent>
            </Select>
          </div>
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <Button onClick={() => onConfirm(destTabId, location)} disabled={!destTabId}>
            Convert
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

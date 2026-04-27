import { memo } from "react";
import type React from "react";
import { FileCommandPopup } from "@/components/FileCommandPopup";
import { HistorySearchPopup } from "@/components/HistorySearchPopup";
import { PathCompletionPopup } from "@/components/PathCompletionPopup";
import { SlashCommandPopup } from "@/components/SlashCommandPopup";
import { ToolSearchPopup } from "@/components/ToolSearchPopup/ToolSearchPopup";
import type { HistoryMatch } from "@/hooks/useHistorySearch";
import type { SlashCommand } from "@/hooks/useSlashCommands";
import type { ToolConfig } from "@/lib/pentest/types";
import type { FileInfo, PathCompletion } from "@/lib/tauri";

interface InputPopupsProps {
  containerRef: React.RefObject<HTMLDivElement | null>;
  showHistorySearch: boolean;
  setShowHistorySearch: (open: boolean) => void;
  historyMatches: HistoryMatch[];
  historySelectedIndex: number;
  historySearchQuery: string;
  onHistorySelect: (match: HistoryMatch) => void;
  showPathPopup: boolean;
  setShowPathPopup: (open: boolean) => void;
  pathCompletions: PathCompletion[];
  pathTotalCount: number;
  pathSelectedIndex: number;
  onPathSelect: (completion: PathCompletion) => void;
  showSlashPopup: boolean;
  setShowSlashPopup: (open: boolean) => void;
  filteredSlashCommands: SlashCommand[];
  slashSelectedIndex: number;
  onSlashSelect: (cmd: SlashCommand) => void;
  showFilePopup: boolean;
  setShowFilePopup: (open: boolean) => void;
  files: FileInfo[];
  fileSelectedIndex: number;
  onFileSelect: (file: FileInfo) => void;
  showToolPopup: boolean;
  setShowToolPopup: (open: boolean) => void;
  toolMatches: ToolConfig[];
  toolSelectedIndex: number;
  onToolSelect: (tool: ToolConfig) => void;
}

export const InputPopups = memo(function InputPopups(props: InputPopupsProps) {
  return (
    <>
      <HistorySearchPopup
        open={props.showHistorySearch}
        onOpenChange={props.setShowHistorySearch}
        matches={props.historyMatches}
        selectedIndex={props.historySelectedIndex}
        searchQuery={props.historySearchQuery}
        onSelect={props.onHistorySelect}
        containerRef={props.containerRef}
      />
      <PathCompletionPopup
        open={props.showPathPopup}
        onOpenChange={props.setShowPathPopup}
        completions={props.pathCompletions}
        totalCount={props.pathTotalCount}
        selectedIndex={props.pathSelectedIndex}
        onSelect={props.onPathSelect}
        containerRef={props.containerRef}
      />
      <SlashCommandPopup
        open={props.showSlashPopup}
        onOpenChange={props.setShowSlashPopup}
        commands={props.filteredSlashCommands}
        selectedIndex={props.slashSelectedIndex}
        onSelect={props.onSlashSelect}
        containerRef={props.containerRef}
      />
      <FileCommandPopup
        open={props.showFilePopup}
        onOpenChange={props.setShowFilePopup}
        files={props.files}
        selectedIndex={props.fileSelectedIndex}
        onSelect={props.onFileSelect}
        containerRef={props.containerRef}
      />
      <ToolSearchPopup
        open={props.showToolPopup && props.toolMatches.length > 0}
        onOpenChange={props.setShowToolPopup}
        tools={props.toolMatches}
        selectedIndex={props.toolSelectedIndex}
        onSelect={props.onToolSelect}
        containerRef={props.containerRef}
      />
    </>
  );
});

import { memo, useEffect, useRef } from "react";
import { Milkdown, MilkdownProvider, useEditor } from "@milkdown/react";
import { CrepeBuilder } from "@milkdown/crepe";
import { toolbar } from "@milkdown/crepe/feature/toolbar";
import { blockEdit } from "@milkdown/crepe/feature/block-edit";
import { placeholder } from "@milkdown/crepe/feature/placeholder";
import { listItem } from "@milkdown/crepe/feature/list-item";
import { linkTooltip } from "@milkdown/crepe/feature/link-tooltip";
import { cursor } from "@milkdown/crepe/feature/cursor";
import { codeMirror } from "@milkdown/crepe/feature/code-mirror";
import { table } from "@milkdown/crepe/feature/table";
import { vscodeDark } from "@uiw/codemirror-theme-vscode";
import { languages } from "@codemirror/language-data";

import "@milkdown/crepe/theme/common/style.css";
import "@milkdown/crepe/theme/classic-dark.css";

interface MarkdownEditorProps {
  editorKey: string;
  value: string;
  onChange: (value: string) => void;
  className?: string;
}

function MilkdownEditorInner({
  value,
  onChange,
}: { value: string; onChange: (v: string) => void }) {
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;

  useEditor((root) => {
    return new CrepeBuilder({ root, defaultValue: value })
      .addFeature(toolbar)
      .addFeature(blockEdit)
      .addFeature(placeholder, { text: "Start writing...", mode: "block" })
      .addFeature(listItem)
      .addFeature(linkTooltip)
      .addFeature(cursor)
      .addFeature(codeMirror, { theme: vscodeDark, languages })
      .addFeature(table)
      .on((listener) => {
        listener.markdownUpdated((_ctx, markdown) => {
          onChangeRef.current(markdown);
        });
      });
  }, []);

  return <Milkdown />;
}

export const MarkdownEditor = memo(function MarkdownEditor({
  editorKey,
  value,
  onChange,
  className,
}: MarkdownEditorProps) {
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    const handleClick = (e: MouseEvent) => {
      const target = e.target as HTMLElement;
      const clickedPicker = target.closest(".milkdown-code-block .language-button");
      if (!clickedPicker) return;

      const allPickers = el.querySelectorAll(".milkdown-code-block .list-wrapper");
      const clickedBlock = target.closest(".milkdown-code-block");
      for (const picker of allPickers) {
        if (picker.closest(".milkdown-code-block") !== clickedBlock) {
          picker.classList.add("hidden");
        }
      }
    };

    el.addEventListener("click", handleClick, true);
    return () => el.removeEventListener("click", handleClick, true);
  }, []);

  return (
    <div
      ref={containerRef}
      className={`milkdown-skill-editor h-full overflow-auto ${className ?? ""}`}
    >
      <MilkdownProvider key={editorKey}>
        <MilkdownEditorInner value={value} onChange={onChange} />
      </MilkdownProvider>
    </div>
  );
});

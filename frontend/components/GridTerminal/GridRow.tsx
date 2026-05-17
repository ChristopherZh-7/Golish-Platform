/**
 * One row of the virtual terminal grid.
 *
 * Renders each non-wide-spacer cell as a `<span>` whose `className`
 * encodes its visual attributes (`fg-default`, `fg-rgb`, `attr-bold`,
 * etc.). The actual colour resolution and font choice live in
 * `frontend/styles/grid-terminal.css` — keeping the JSX dumb means a
 * row only re-renders when its `cells` array reference changes.
 */

import { memo } from "react";
import type { GridCellPayload, GridColor } from "@/lib/events/payloads";

interface GridRowProps {
  cells: GridCellPayload[];
  cursorX: number;
  cursorVisible: boolean;
}

const ATTR_BOLD = 1 << 0;
const ATTR_ITALIC = 1 << 1;
const ATTR_UNDERLINE = 1 << 2;
const ATTR_INVERSE = 1 << 3;
const ATTR_STRIKETHROUGH = 1 << 4;
const ATTR_DIM = 1 << 5;
const ATTR_HIDDEN = 1 << 6;
const ATTR_BLINK = 1 << 7;

function colorClass(color: GridColor, role: "fg" | "bg"): string {
  switch (color.kind) {
    case "default":
      return `gt-${role}-default`;
    case "indexed":
      // Indexed palette resolved in CSS via `data-` attribute; class
      // alone can't carry the numeric value so we put it on `style`
      // below for indexed cells.
      return `gt-${role}-indexed`;
    case "rgb":
      return `gt-${role}-rgb`;
  }
}

function buildCellStyle(cell: GridCellPayload): React.CSSProperties | undefined {
  const style: React.CSSProperties = {};
  if (cell.fg.kind === "rgb") {
    style.color = `#${cell.fg.value.toString(16).padStart(6, "0")}`;
  } else if (cell.fg.kind === "indexed") {
    style.color = `var(--gt-palette-${cell.fg.value}, currentColor)`;
  }
  if (cell.bg.kind === "rgb") {
    style.backgroundColor = `#${cell.bg.value.toString(16).padStart(6, "0")}`;
  } else if (cell.bg.kind === "indexed") {
    style.backgroundColor = `var(--gt-palette-${cell.bg.value}, transparent)`;
  }
  return Object.keys(style).length === 0 ? undefined : style;
}

function buildAttrClasses(attrs: number): string {
  if (attrs === 0) return "";
  const classes: string[] = [];
  if (attrs & ATTR_BOLD) classes.push("gt-bold");
  if (attrs & ATTR_ITALIC) classes.push("gt-italic");
  if (attrs & ATTR_UNDERLINE) classes.push("gt-underline");
  if (attrs & ATTR_INVERSE) classes.push("gt-inverse");
  if (attrs & ATTR_STRIKETHROUGH) classes.push("gt-strikethrough");
  if (attrs & ATTR_DIM) classes.push("gt-dim");
  if (attrs & ATTR_HIDDEN) classes.push("gt-hidden");
  if (attrs & ATTR_BLINK) classes.push("gt-blink");
  return classes.join(" ");
}

export const GridRow = memo(function GridRow({ cells, cursorX, cursorVisible }: GridRowProps) {
  return (
    <div className="gt-row">
      {cells.map((cell, x) => {
        // Wide-char continuation slots — backend emits `\0` so the
        // previous cell already painted the glyph (with width=2).
        if (cell.ch === "\0") return null;
        const isCursor = cursorVisible && x === cursorX;
        const classes = [colorClass(cell.fg, "fg"), colorClass(cell.bg, "bg")];
        const attrClasses = buildAttrClasses(cell.attrs);
        if (attrClasses) classes.push(attrClasses);
        if (isCursor) classes.push("gt-cursor");
        return (
          <span key={x} className={classes.join(" ")} style={buildCellStyle(cell)}>
            {cell.ch === " " ? "\u00a0" : cell.ch}
          </span>
        );
      })}
    </div>
  );
});

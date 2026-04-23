import {
  type ComponentPropsWithoutRef,
  createContext,
  lazy,
  memo,
  type ReactNode,
  Suspense,
  useContext,
  useDeferredValue,
  useMemo,
  useState,
} from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

// Lazy load the SyntaxHighlighter component (~170KB)
// This significantly improves initial page load time
const LazySyntaxHighlighter = lazy(() =>
  import("react-syntax-highlighter").then((mod) => ({
    default: mod.Prism,
  }))
);

// Lazy load the theme (imported separately to avoid blocking)
const getCodeTheme = async () => {
  const { oneDark } = await import("react-syntax-highlighter/dist/esm/styles/prism");
  return {
    ...oneDark,
    'code[class*="language-"]': {
      ...oneDark['code[class*="language-"]'],
      background: "transparent",
    },
    'pre[class*="language-"]': {
      ...oneDark['pre[class*="language-"]'],
      background: "transparent",
    },
  };
};

// Cache the theme once loaded
let cachedCodeTheme: Record<string, unknown> | null = null;
// Start loading the theme immediately on module load
getCodeTheme().then((theme) => {
  cachedCodeTheme = theme;
});

import { ChevronDown, FileCode } from "lucide-react";
import { stripAllAnsi } from "@/lib/ansi";
import { FilePathLink } from "@/components/FilePathLink";
import { useFileIndex } from "@/hooks/useFileIndex";
import type { FileIndex } from "@/lib/fileIndex";
import { detectFilePathsWithIndex } from "@/lib/pathDetection";
import { cn } from "@/lib/utils";
import { CopyButton } from "./CopyButton";

interface MarkdownContextValue {
  sessionId?: string;
  workingDirectory?: string;
  fileIndex?: FileIndex;
}

const MarkdownContext = createContext<MarkdownContextValue>({});

export function useMarkdownContext() {
  return useContext(MarkdownContext);
}

function processTextWithFilePaths(text: string, context: MarkdownContextValue): ReactNode {
  const { sessionId, workingDirectory, fileIndex } = context;

  // If no context or fileIndex available, return text as-is (no links)
  if (!sessionId || !workingDirectory || !fileIndex) {
    return text;
  }

  // Detect file paths in the text
  const detectedPaths = detectFilePathsWithIndex(text, fileIndex);

  // If no paths detected, return text as-is
  if (detectedPaths.length === 0) {
    return text;
  }

  // Split text into segments with file path links
  const segments: ReactNode[] = [];
  let lastIndex = 0;

  for (let idx = 0; idx < detectedPaths.length; idx++) {
    const detected = detectedPaths[idx];

    // Add text before this path
    if (detected.start > lastIndex) {
      segments.push(text.substring(lastIndex, detected.start));
    }

    // Add FilePathLink component
    segments.push(
      <FilePathLink
        key={`path-${idx}`}
        detected={detected}
        workingDirectory={workingDirectory}
        absolutePath={detected.absolutePath}
      >
        {detected.raw}
      </FilePathLink>
    );

    lastIndex = detected.end;
  }

  // Add remaining text
  if (lastIndex < text.length) {
    segments.push(text.substring(lastIndex));
  }

  return <>{segments}</>;
}

interface MarkdownProps {
  content: string;
  className?: string;
  /** Lightweight mode for streaming content - avoids expensive parsing */
  streaming?: boolean;
  sessionId?: string;
  workingDirectory?: string;
}

const LANG_LABELS: Record<string, string> = {
  js: "JavaScript", jsx: "JSX", ts: "TypeScript", tsx: "TSX",
  py: "Python", rb: "Ruby", rs: "Rust", go: "Go", java: "Java",
  sh: "Shell", bash: "Bash", zsh: "Zsh", fish: "Fish",
  css: "CSS", html: "HTML", json: "JSON", yaml: "YAML", yml: "YAML",
  toml: "TOML", xml: "XML", sql: "SQL", md: "Markdown",
  c: "C", cpp: "C++", cs: "C#", swift: "Swift", kt: "Kotlin",
  php: "PHP", lua: "Lua", r: "R", dart: "Dart", zig: "Zig",
  text: "Plain Text",
};

const COLLAPSED_LINE_LIMIT = 8;

function CodeBlockFallback({ code }: { code: string; language: string }) {
  return (
    <pre className="font-mono text-[12px] text-muted-foreground whitespace-pre-wrap break-words px-3 py-2.5 leading-relaxed">
      {code}
    </pre>
  );
}

function SyntaxHighlightedCode({ code, language, ...props }: { code: string; language: string }) {
  const theme = cachedCodeTheme || {};
  return (
    <LazySyntaxHighlighter
      // biome-ignore lint/suspicious/noExplicitAny: SyntaxHighlighter style prop typing is incompatible
      style={theme as any}
      language={language || "text"}
      PreTag="div"
      customStyle={{
        margin: 0,
        padding: "0.625rem 0.75rem",
        background: "transparent",
        fontSize: "12px",
        lineHeight: "1.55",
      }}
      {...props}
    >
      {code}
    </LazySyntaxHighlighter>
  );
}

function CodeBlock({
  inline,
  className,
  children,
  ...props
}: ComponentPropsWithoutRef<"code"> & { inline?: boolean }) {
  const context = useMarkdownContext();
  const match = /language-(\w+)/.exec(className || "");
  const language = match ? match[1] : "";
  const rawCodeString = String(children).replace(/\n$/, "");
  // Strip any ANSI escape sequences that leaked into markdown content
  const codeString = stripAllAnsi(rawCodeString);
  const [expanded, setExpanded] = useState(false);

  if (!inline && (match || codeString.includes("\n"))) {
    const langLabel = LANG_LABELS[language] || language.toUpperCase() || "CODE";
    const lineCount = codeString.split("\n").length;
    const isLong = lineCount > COLLAPSED_LINE_LIMIT;

    return (
      <div className="my-3 rounded-lg border border-border/40 bg-[var(--background)] overflow-hidden group">
        {/* Header bar */}
        <div className="flex items-center justify-between px-3 py-1 bg-muted/40 border-b border-border/30">
          <div className="flex items-center gap-1.5">
            <FileCode className="w-3 h-3 text-muted-foreground/60" />
            <span className="text-[10px] font-medium text-muted-foreground/70">{langLabel}</span>
            {isLong && (
              <span className="text-[9px] text-muted-foreground/40 ml-1">{lineCount} lines</span>
            )}
          </div>
          <div className="flex items-center gap-1">
            <CopyButton
              content={codeString}
              className="opacity-0 group-hover:opacity-100 transition-opacity"
            />
            {isLong && (
              <button
                type="button"
                className="p-0.5 rounded hover:bg-muted/60 transition-colors"
                onClick={() => setExpanded(!expanded)}
                title={expanded ? "Collapse" : "Expand"}
              >
                <ChevronDown className={cn(
                  "w-3.5 h-3.5 text-muted-foreground/50 transition-transform",
                  expanded && "rotate-180",
                )} />
              </button>
            )}
          </div>
        </div>
        {/* Code body */}
        <div className={cn(
          "overflow-x-auto relative",
          isLong && !expanded && "max-h-[180px] overflow-hidden",
        )}>
          <Suspense fallback={<CodeBlockFallback code={codeString} language={language} />}>
            <SyntaxHighlightedCode code={codeString} language={language} {...props} />
          </Suspense>
          {isLong && !expanded && (
            <div className="absolute bottom-0 left-0 right-0 h-8 bg-gradient-to-t from-[var(--background)] to-transparent pointer-events-none" />
          )}
        </div>
        {/* Expand footer */}
        {isLong && !expanded && (
          <button
            type="button"
            className="w-full flex items-center justify-center gap-1 py-1 text-[10px] text-muted-foreground/50 hover:text-muted-foreground/70 hover:bg-muted/20 transition-colors border-t border-border/20"
            onClick={() => setExpanded(true)}
          >
            <ChevronDown className="w-3 h-3" />
            Show all {lineCount} lines
          </button>
        )}
      </div>
    );
  }

  // For inline code, try to detect file paths
  const processedContent = processTextWithFilePaths(codeString, context);
  const hasFileLinks = processedContent !== codeString;

  return (
    <code
      className={cn(
        "px-1.5 py-0.5 rounded bg-background border border-[var(--border-medium)] text-foreground/80 font-mono text-[0.85em]",
        // Remove whitespace-nowrap if we have file links to allow proper styling
        !hasFileLinks && "whitespace-nowrap",
        className
      )}
      {...props}
    >
      {processedContent}
    </code>
  );
}

// Stable reference — never changes between renders
const remarkPlugins = [remarkGfm];

export const Markdown = memo(function Markdown({
  content,
  className,
  streaming,
  sessionId,
  workingDirectory,
}: MarkdownProps) {
  const fileIndex = useFileIndex(workingDirectory);
  // During streaming, defer markdown parsing so React can skip intermediate
  // renders and keep the UI responsive even on long responses.
  const deferredContent = useDeferredValue(content);
  const renderedContent = streaming ? deferredContent : content;

  const contextValue = useMemo(
    () => ({ sessionId, workingDirectory, fileIndex: fileIndex ?? undefined }),
    [sessionId, workingDirectory, fileIndex]
  );

  // Memoize components so ReactMarkdown doesn't re-parse when only the parent
  // re-renders but renderedContent hasn't changed yet (deferred).
  const components = useMemo(
    () => ({
      code: CodeBlock,
      // Headings
      h1: ({ children }: { children?: ReactNode }) => (
        <h1 className="text-base font-bold text-foreground mt-4 mb-2 first:mt-0 pb-1.5 border-b border-[var(--border-medium)]">
          {children}
        </h1>
      ),
      h2: ({ children }: { children?: ReactNode }) => (
        <h2 className="text-[13px] font-bold text-accent mt-3 mb-2 first:mt-0 pb-1.5 border-b border-[var(--border-subtle)] flex items-center gap-1.5">
          <span className="w-0.5 h-4 bg-accent rounded-full" />
          {children}
        </h2>
      ),
      h3: ({ children }: { children?: ReactNode }) => (
        <h3 className="text-[12.5px] font-semibold text-muted-foreground mt-3 mb-1.5 first:mt-0 pl-2.5 border-l-2 border-accent">
          {children}
        </h3>
      ),
      // Paragraphs
      p: ({ children }: { children?: ReactNode }) => (
        <p className="text-foreground mb-2 last:mb-0 leading-relaxed">
          {typeof children === "string"
            ? processTextWithFilePaths(children, contextValue)
            : children}
        </p>
      ),
      // Lists
      ul: ({ children }: { children?: ReactNode }) => (
        <ul className="list-disc list-outside text-foreground mb-2 space-y-1 pl-5">{children}</ul>
      ),
      ol: ({ children }: { children?: ReactNode }) => (
        <ol className="list-decimal list-outside text-foreground mb-2 space-y-1 pl-5">
          {children}
        </ol>
      ),
      li: ({ children }: { children?: ReactNode }) => (
        <li className="text-foreground leading-relaxed">
          {typeof children === "string"
            ? processTextWithFilePaths(children, contextValue)
            : children}
        </li>
      ),
      // Links
      a: ({ href, children }: { href?: string; children?: ReactNode }) => (
        <a
          href={href}
          target="_blank"
          rel="noopener noreferrer"
          className="text-primary underline decoration-primary/30 hover:decoration-primary/70 transition-colors"
        >
          {children}
        </a>
      ),
      // Blockquotes
      blockquote: ({ children }: { children?: ReactNode }) => (
        <blockquote className="border-l-4 border-accent bg-[var(--accent-dim)] pl-4 py-2 my-3 text-muted-foreground italic rounded-r">
          {typeof children === "string"
            ? processTextWithFilePaths(children, contextValue)
            : children}
        </blockquote>
      ),
      // Horizontal rule
      hr: () => <hr className="my-4 border-[var(--border-medium)]" />,
      // Strong and emphasis
      strong: ({ children }: { children?: ReactNode }) => (
        <strong className="font-bold text-foreground">{children}</strong>
      ),
      em: ({ children }: { children?: ReactNode }) => (
        <em className="italic text-[var(--success)]">{children}</em>
      ),
      // Tables
      table: ({ children }: { children?: ReactNode }) => (
        <div className="overflow-x-auto my-3">
          <table className="border-collapse text-[13px]">{children}</table>
        </div>
      ),
      thead: ({ children }: { children?: ReactNode }) => (
        <thead className="bg-muted/50 border-b border-[var(--border-subtle)]">{children}</thead>
      ),
      tbody: ({ children }: { children?: ReactNode }) => <tbody>{children}</tbody>,
      tr: ({ children }: { children?: ReactNode }) => (
        <tr className="border-b border-[var(--border-subtle)] last:border-b-0 [tbody>&]:hover:bg-muted/30">
          {children}
        </tr>
      ),
      th: ({ children }: { children?: ReactNode }) => (
        <th className="px-3 py-1.5 text-left text-foreground/80 font-medium text-[12px] uppercase tracking-wide">
          {typeof children === "string"
            ? processTextWithFilePaths(children, contextValue)
            : children}
        </th>
      ),
      td: ({ children }: { children?: ReactNode }) => (
        <td className="px-3 py-2 text-muted-foreground">
          {typeof children === "string"
            ? processTextWithFilePaths(children, contextValue)
            : children}
        </td>
      ),
    }),
    [contextValue]
  );

  // Memoize the ReactMarkdown output so remark parsing only runs when the
  // deferred content actually changes, not on every parent re-render.
  const markdownElement = useMemo(
    () => (
      <ReactMarkdown remarkPlugins={remarkPlugins} components={components}>
        {renderedContent}
      </ReactMarkdown>
    ),
    [renderedContent, components]
  );

  return (
    <MarkdownContext.Provider value={contextValue}>
      <div
        className={cn(
          "max-w-none break-words overflow-hidden text-foreground leading-relaxed",
          className
        )}
      >
        {markdownElement}
      </div>
    </MarkdownContext.Provider>
  );
});

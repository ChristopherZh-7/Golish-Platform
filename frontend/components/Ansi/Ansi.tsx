import Anser from "anser";
import { useMemo } from "react";

interface AnsiProps {
  children: string;
  useClasses?: boolean;
}

/**
 * Renders ANSI escape codes as colored HTML.
 * Drop-in replacement for ansi-to-react that works with React 19.
 */
export function Ansi({ children, useClasses = false }: AnsiProps) {
  const html = useMemo(() => {
    if (!children) return "";
    
    // Convert ANSI to HTML using anser
    const anserOutput = Anser.ansiToHtml(children, {
      use_classes: useClasses,
      escapeXML: true,
    });
    
    return anserOutput;
  }, [children, useClasses]);

  return <span dangerouslySetInnerHTML={{ __html: html }} />;
}

export default Ansi;

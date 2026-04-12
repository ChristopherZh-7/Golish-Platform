import { Component, type ReactNode } from "react";
import { logger } from "@/lib/logger";

interface ErrorBoundaryProps {
  children: ReactNode;
  /** Optional fallback to render when an error occurs. If not provided, children continue to render after error is logged. */
  fallback?: ReactNode;
  /** Called when an error is caught */
  onError?: (error: Error, errorInfo: React.ErrorInfo) => void;
}

interface ErrorBoundaryState {
  hasError: boolean;
  error: Error | null;
  crashCount: number;
}

const MAX_AUTO_RECOVERY = 3;
const CRASH_WINDOW_MS = 5000;

/**
 * Error boundary component that catches errors in its child component tree.
 *
 * Unlike typical error boundaries that show a fallback UI, this one logs the error
 * and continues rendering children by default. This allows the app to keep working
 * even when individual components throw errors.
 *
 * To show a fallback UI instead, pass the `fallback` prop.
 */
export class ErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  private firstCrashTime = 0;

  constructor(props: ErrorBoundaryProps) {
    super(props);
    this.state = { hasError: false, error: null, crashCount: 0 };
  }

  static getDerivedStateFromError(error: Error): Partial<ErrorBoundaryState> {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, errorInfo: React.ErrorInfo) {
    logger.error("[ErrorBoundary] Caught error:", error.message, {
      componentStack: errorInfo.componentStack,
      stack: error.stack,
    });

    this.props.onError?.(error, errorInfo);

    const now = Date.now();
    if (now - this.firstCrashTime > CRASH_WINDOW_MS) {
      this.firstCrashTime = now;
      this.setState({ crashCount: 1 });
    } else {
      this.setState((prev) => ({ crashCount: prev.crashCount + 1 }));
    }

    if (!this.props.fallback && this.state.crashCount < MAX_AUTO_RECOVERY) {
      setTimeout(() => {
        this.setState({ hasError: false, error: null });
      }, 100);
    }
  }

  render() {
    if (this.state.hasError) {
      if (this.props.fallback) return this.props.fallback;
      if (this.state.crashCount >= MAX_AUTO_RECOVERY) {
        return (
          <div style={{ padding: 24, color: "#888", fontSize: 13 }}>
            <p>A component crashed repeatedly. Reload the app to retry.</p>
            <pre style={{ fontSize: 11, marginTop: 8, color: "#666" }}>{this.state.error?.message}</pre>
          </div>
        );
      }
    }
    return this.props.children;
  }
}

/**
 * Sets up global error handlers for uncaught errors and unhandled promise rejections.
 * Call this once at app startup (in main.tsx).
 *
 * These handlers log errors but don't interrupt the app, allowing it to continue
 * functioning even when errors occur.
 */
export function setupGlobalErrorHandlers(): void {
  // Handle uncaught errors
  window.onerror = (message, source, lineno, colno, error) => {
    const msg = String(message);
    const src = typeof source === "string" ? source : "";

    // xterm.js renderer race condition during WebGL swap — harmless, downgrade to debug
    if (src.includes("@xterm") && msg.includes("_renderer")) {
      logger.debug("[Terminal] Renderer initialization race (harmless):", msg);
      return true;
    }

    logger.error("[GlobalError] Uncaught error:", {
      message: msg,
      source,
      lineno,
      colno,
      error: error?.message,
      stack: error?.stack,
    });

    // Return true to prevent the browser's default error handling (which would show an error overlay)
    // This allows the app to continue running
    return true;
  };

  // Handle unhandled promise rejections
  window.onunhandledrejection = (event) => {
    logger.error("[GlobalError] Unhandled promise rejection:", {
      reason: event.reason instanceof Error ? event.reason.message : String(event.reason),
      stack: event.reason instanceof Error ? event.reason.stack : undefined,
    });

    // Prevent the browser from logging the rejection to console (we already logged it)
    event.preventDefault();
  };
}

import { normalizeExecutionModeId } from "@/lib/ai";
import { suppressTerminalAutoFocus } from "@/lib/terminal/terminalAutoFocus";
import { useStore } from "@/store";

type ModeSetter = (mode: string) => void;

function resolveMode(mode: string | (() => string)): string {
  return typeof mode === "function" ? mode() : mode;
}

/**
 * Chat tab switches need the associated terminal to become the active backing
 * session for context/tools, but the DOM focus should stay in the chat panel.
 */
export function activateConversationTerminalFromChat(
  activeConvId: string | null,
  opts: {
    setChatExecutionMode: ModeSetter;
    emptyExecutionMode: string | (() => string);
  }
): string | null {
  if (!activeConvId) return null;

  const store = useStore.getState();
  const terminals = store.conversationTerminals[activeConvId];
  if (terminals && terminals.length > 0) {
    const firstTerminal = terminals[0];
    if (store.sessions[firstTerminal]) {
      suppressTerminalAutoFocus(firstTerminal);
      if (store.activeSessionId !== firstTerminal) {
        store.setActiveSession(firstTerminal);
      }
    }
    for (const tid of terminals) {
      const em = store.sessions[tid]?.executionMode;
      if (em && em !== "chat") {
        const mode = normalizeExecutionModeId(em);
        if (mode !== em) store.setExecutionMode(tid, mode);
        opts.setChatExecutionMode(mode);
        break;
      }
    }
    return firstTerminal;
  }

  opts.setChatExecutionMode(resolveMode(opts.emptyExecutionMode));
  return null;
}

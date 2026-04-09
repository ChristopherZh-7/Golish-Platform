import { HelpCircle, KeyRound, List, MessageSquare, ShieldQuestion } from "lucide-react";
import { useCallback, useState } from "react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { respondToToolApproval } from "@/lib/ai";
import { logger } from "@/lib/logger";
import { cn } from "@/lib/utils";
import type { AskHumanRequest } from "@/store";
import { usePendingAskHuman, useStore } from "@/store";

interface AskHumanDialogProps {
  sessionId: string;
}

const INPUT_TYPE_CONFIG: Record<
  AskHumanRequest["inputType"],
  { icon: typeof HelpCircle; label: string; color: string }
> = {
  credentials: { icon: KeyRound, label: "Credentials Required", color: "text-[#e0af68]" },
  choice: { icon: List, label: "Choice Required", color: "text-[#7aa2f7]" },
  freetext: { icon: MessageSquare, label: "Input Required", color: "text-[#9ece6a]" },
  confirmation: { icon: ShieldQuestion, label: "Confirmation Required", color: "text-[#bb9af7]" },
};

export function AskHumanDialog({ sessionId }: AskHumanDialogProps) {
  const request = usePendingAskHuman(sessionId);
  const clearPendingAskHuman = useStore((state) => state.clearPendingAskHuman);

  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [freetext, setFreetext] = useState("");
  const [selectedOptions, setSelectedOptions] = useState<Set<string>>(new Set());
  const [isSubmitting, setIsSubmitting] = useState(false);

  const resetForm = useCallback(() => {
    setUsername("");
    setPassword("");
    setFreetext("");
    setSelectedOptions(new Set());
    setIsSubmitting(false);
  }, []);

  const handleSubmit = useCallback(async () => {
    if (!request) return;
    setIsSubmitting(true);

    let response = "";
    switch (request.inputType) {
      case "credentials":
        response = JSON.stringify({ username, password });
        break;
      case "choice":
        response = Array.from(selectedOptions).join(", ");
        break;
      case "freetext":
        response = freetext;
        break;
      case "confirmation":
        response = "yes";
        break;
    }

    try {
      await respondToToolApproval(sessionId, {
        request_id: request.requestId,
        approved: true,
        reason: response,
        remember: false,
        always_allow: false,
      });
      clearPendingAskHuman(sessionId);
      resetForm();
    } catch (err) {
      logger.error("Failed to respond to ask_human:", err);
      setIsSubmitting(false);
    }
  }, [request, sessionId, username, password, freetext, selectedOptions, clearPendingAskHuman, resetForm]);

  const handleSkip = useCallback(async () => {
    if (!request) return;
    setIsSubmitting(true);

    try {
      await respondToToolApproval(sessionId, {
        request_id: request.requestId,
        approved: false,
        reason: undefined,
        remember: false,
        always_allow: false,
      });
      clearPendingAskHuman(sessionId);
      resetForm();
    } catch (err) {
      logger.error("Failed to skip ask_human:", err);
      setIsSubmitting(false);
    }
  }, [request, sessionId, clearPendingAskHuman, resetForm]);

  const handleAbort = useCallback(async () => {
    if (!request) return;
    setIsSubmitting(true);

    try {
      await respondToToolApproval(sessionId, {
        request_id: request.requestId,
        approved: false,
        reason: "__abort__",
        remember: false,
        always_allow: false,
      });
      clearPendingAskHuman(sessionId);
      resetForm();
    } catch (err) {
      logger.error("Failed to abort ask_human:", err);
      setIsSubmitting(false);
    }
  }, [request, sessionId, clearPendingAskHuman, resetForm]);

  if (!request) return null;

  const config = INPUT_TYPE_CONFIG[request.inputType];
  const Icon = config.icon;

  return (
    <Dialog open={true} onOpenChange={() => {}}>
      <DialogContent className="sm:max-w-[480px] bg-[#1a1b26] border-[#27293d] text-[#c0caf5]">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2 text-[#c0caf5]">
            <Icon className={cn("w-5 h-5", config.color)} />
            {config.label}
          </DialogTitle>
          <DialogDescription className="text-[#565f89] text-sm">
            The AI agent needs your input to continue.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 py-2">
          <div className="rounded-lg bg-[#16161e] border border-[#27293d] p-4">
            <p className="text-sm text-[#c0caf5] whitespace-pre-wrap">{request.question}</p>
            {request.context && (
              <p className="mt-2 text-xs text-[#565f89] italic">{request.context}</p>
            )}
          </div>

          {request.inputType === "credentials" && (
            <div className="space-y-3">
              <div>
                <label className="text-xs text-[#565f89] mb-1 block">Username</label>
                <input
                  type="text"
                  value={username}
                  onChange={(e) => setUsername(e.target.value)}
                  className="w-full px-3 py-2 rounded-md bg-[#16161e] border border-[#27293d] text-[#c0caf5] text-sm focus:outline-none focus:border-[#7aa2f7]"
                  placeholder="Enter username..."
                  autoFocus
                />
              </div>
              <div>
                <label className="text-xs text-[#565f89] mb-1 block">Password</label>
                <input
                  type="password"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  className="w-full px-3 py-2 rounded-md bg-[#16161e] border border-[#27293d] text-[#c0caf5] text-sm focus:outline-none focus:border-[#7aa2f7]"
                  placeholder="Enter password..."
                  onKeyDown={(e) => e.key === "Enter" && handleSubmit()}
                />
              </div>
            </div>
          )}

          {request.inputType === "choice" && (
            <div className="space-y-2">
              {request.options.map((option) => (
                <button
                  key={option}
                  type="button"
                  onClick={() => {
                    setSelectedOptions((prev) => {
                      const next = new Set(prev);
                      if (next.has(option)) next.delete(option);
                      else next.add(option);
                      return next;
                    });
                  }}
                  className={cn(
                    "w-full text-left px-3 py-2 rounded-md border text-sm transition-colors",
                    selectedOptions.has(option)
                      ? "bg-[#7aa2f7]/15 border-[#7aa2f7]/50 text-[#7aa2f7]"
                      : "bg-[#16161e] border-[#27293d] text-[#c0caf5] hover:border-[#414868]"
                  )}
                >
                  {option}
                </button>
              ))}
            </div>
          )}

          {request.inputType === "freetext" && (
            <textarea
              value={freetext}
              onChange={(e) => setFreetext(e.target.value)}
              className="w-full px-3 py-2 rounded-md bg-[#16161e] border border-[#27293d] text-[#c0caf5] text-sm focus:outline-none focus:border-[#7aa2f7] min-h-[80px] resize-y"
              placeholder="Type your response..."
              autoFocus
            />
          )}

          {request.inputType === "confirmation" && (
            <p className="text-sm text-[#565f89]">
              Click "Confirm" to approve or "Skip" to decline.
            </p>
          )}
        </div>

        <DialogFooter className="flex gap-2 sm:gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={handleAbort}
            disabled={isSubmitting}
            className="border-[#f7768e]/30 text-[#f7768e] hover:bg-[#f7768e]/10"
          >
            Abort Task
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={handleSkip}
            disabled={isSubmitting}
            className="border-[#414868] text-[#565f89] hover:bg-[#27293d]"
          >
            Skip
          </Button>
          <Button
            size="sm"
            onClick={handleSubmit}
            disabled={isSubmitting}
            className="bg-[#7aa2f7] text-[#1a1b26] hover:bg-[#7aa2f7]/90"
          >
            {request.inputType === "confirmation" ? "Confirm" : "Submit"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

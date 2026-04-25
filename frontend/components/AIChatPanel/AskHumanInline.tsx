import { KeyRound, List, MessageSquare, ShieldQuestion } from "lucide-react";
import { useState } from "react";
import { cn } from "@/lib/utils";

export interface AskHumanState {
  requestId: string;
  sessionId: string;
  question: string;
  inputType: "credentials" | "choice" | "freetext" | "confirmation";
  options: string[];
  context: string;
}

const INPUT_TYPE_ICONS: Record<string, typeof KeyRound> = {
  credentials: KeyRound,
  choice: List,
  freetext: MessageSquare,
  confirmation: ShieldQuestion,
};

export function AskHumanInline({
  request,
  onSubmit,
  onSkip,
}: {
  request: AskHumanState;
  onSubmit: (response: string) => void;
  onSkip: () => void;
}) {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [freetext, setFreetext] = useState("");
  const [selectedOptions, setSelectedOptions] = useState<Set<string>>(new Set());

  const Icon = INPUT_TYPE_ICONS[request.inputType] || MessageSquare;

  const handleSubmit = () => {
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
    onSubmit(response);
  };

  return (
    <div className="mx-4 my-2 rounded-lg border border-[#e0af68]/30 bg-[#e0af68]/5 p-3">
      <div className="flex items-center gap-2 text-[12px] font-medium text-[#e0af68] mb-2">
        <Icon className="w-3.5 h-3.5" />
        AI Needs Your Input
      </div>
      <p className="text-[13px] text-foreground mb-2 whitespace-pre-wrap">{request.question}</p>
      {request.context && (
        <p className="text-[11px] text-muted-foreground/60 mb-2 italic">{request.context}</p>
      )}

      {request.inputType === "credentials" && (
        <div className="space-y-2 mb-2">
          <input
            type="text"
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            className="w-full px-2.5 py-1.5 rounded-md bg-background border border-border/50 text-[12px] focus:outline-none focus:border-accent"
            placeholder="Username..."
          />
          <input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            className="w-full px-2.5 py-1.5 rounded-md bg-background border border-border/50 text-[12px] focus:outline-none focus:border-accent"
            placeholder="Password..."
            onKeyDown={(e) => e.key === "Enter" && handleSubmit()}
          />
        </div>
      )}

      {request.inputType === "choice" && (
        <div className="space-y-1 mb-2">
          {request.options.map((opt) => (
            <button
              key={opt}
              type="button"
              onClick={() =>
                setSelectedOptions((prev) => {
                  const next = new Set(prev);
                  if (next.has(opt)) next.delete(opt);
                  else next.add(opt);
                  return next;
                })
              }
              className={cn(
                "w-full text-left px-2.5 py-1.5 rounded-md border text-[12px] transition-colors",
                selectedOptions.has(opt)
                  ? "bg-accent/10 border-accent/50 text-accent"
                  : "bg-background border-border/50 hover:border-muted-foreground/30"
              )}
            >
              {opt}
            </button>
          ))}
        </div>
      )}

      {request.inputType === "freetext" && (
        <textarea
          value={freetext}
          onChange={(e) => setFreetext(e.target.value)}
          className="w-full px-2.5 py-1.5 rounded-md bg-background border border-border/50 text-[12px] focus:outline-none focus:border-accent min-h-[60px] resize-y mb-2"
          placeholder="Type your response..."
        />
      )}

      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={handleSubmit}
          className="px-3 py-1 text-[11px] rounded-md bg-accent text-accent-foreground hover:bg-accent/80 font-medium transition-colors"
        >
          {request.inputType === "confirmation" ? "Confirm" : "Submit"}
        </button>
        <button
          type="button"
          onClick={onSkip}
          className="px-3 py-1 text-[11px] rounded-md border border-border/50 text-muted-foreground hover:bg-muted/50 transition-colors"
        >
          Skip
        </button>
      </div>
    </div>
  );
}

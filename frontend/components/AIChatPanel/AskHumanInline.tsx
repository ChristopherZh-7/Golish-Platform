import { KeyRound, List, MessageSquare, Pencil, ShieldQuestion } from "lucide-react";
import { useState } from "react";

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

/** A-Z badges for the first 26 options, then 1-based numbers as a fallback. */
function optionLabel(index: number): string {
  return index < 26 ? String.fromCharCode(65 + index) : String(index + 1);
}

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
  // Cursor-style quick replies: each option submits on a single click. The
  // "Other" affordance reveals a free-text field for a custom answer. When a
  // choice request ships with no options we open that field straight away so the
  // user is never stuck with a dead end.
  const [showOther, setShowOther] = useState(
    request.inputType === "choice" && request.options.length === 0
  );
  const [otherText, setOtherText] = useState("");

  const Icon = INPUT_TYPE_ICONS[request.inputType] || MessageSquare;

  const submitOther = () => {
    const trimmed = otherText.trim();
    if (trimmed) onSubmit(trimmed);
  };

  const handleSubmit = () => {
    switch (request.inputType) {
      case "credentials":
        onSubmit(JSON.stringify({ username, password }));
        break;
      case "freetext":
        onSubmit(freetext);
        break;
      case "confirmation":
        onSubmit("yes");
        break;
    }
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
          {request.options.map((opt, i) => (
            <button
              key={opt}
              type="button"
              onClick={() => onSubmit(opt)}
              className="group w-full text-left px-2.5 py-1.5 rounded-md border border-border/50 bg-background text-[12px] flex items-center gap-2 hover:border-accent/50 hover:bg-accent/10 transition-colors"
            >
              <span className="flex h-4 w-4 flex-shrink-0 items-center justify-center rounded border border-border/60 text-[10px] font-semibold text-muted-foreground group-hover:border-accent/50 group-hover:text-accent">
                {optionLabel(i)}
              </span>
              <span className="flex-1">{opt}</span>
            </button>
          ))}

          {showOther ? (
            <div className="flex items-center gap-1.5 pt-0.5">
              <input
                type="text"
                // biome-ignore lint/a11y/noAutofocus: focus the field the user just revealed
                autoFocus
                value={otherText}
                onChange={(e) => setOtherText(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && submitOther()}
                className="flex-1 px-2.5 py-1.5 rounded-md bg-background border border-border/50 text-[12px] focus:outline-none focus:border-accent"
                placeholder="Type your own answer..."
              />
              <button
                type="button"
                onClick={submitOther}
                disabled={!otherText.trim()}
                className="px-3 py-1.5 text-[11px] rounded-md bg-accent text-accent-foreground hover:bg-accent/80 font-medium transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
              >
                Send
              </button>
            </div>
          ) : (
            <button
              type="button"
              onClick={() => setShowOther(true)}
              className="w-full text-left px-2.5 py-1.5 rounded-md border border-dashed border-border/50 text-[12px] text-muted-foreground flex items-center gap-2 hover:border-accent/40 hover:text-foreground transition-colors"
            >
              <Pencil className="w-3 h-3 flex-shrink-0" />
              Other (type your own)...
            </button>
          )}
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
        {/* Choice options self-submit on click, so no generic Submit button there. */}
        {request.inputType !== "choice" && (
          <button
            type="button"
            onClick={handleSubmit}
            className="px-3 py-1 text-[11px] rounded-md bg-accent text-accent-foreground hover:bg-accent/80 font-medium transition-colors"
          >
            {request.inputType === "confirmation" ? "Confirm" : "Submit"}
          </button>
        )}
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

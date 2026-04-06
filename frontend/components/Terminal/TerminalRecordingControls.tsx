import { memo, useCallback, useEffect, useState } from "react";
import { Circle, Square, Clock } from "lucide-react";
import { cn } from "@/lib/utils";
import {
  isRecording,
  startRecording,
  stopRecording,
} from "@/lib/terminal/recording";

interface Props {
  sessionId: string;
  cols: number;
  rows: number;
}

export const TerminalRecordingControls = memo(function TerminalRecordingControls({
  sessionId,
  cols,
  rows,
}: Props) {
  const [recording, setRecording] = useState(false);
  const [elapsed, setElapsed] = useState(0);

  useEffect(() => {
    setRecording(isRecording(sessionId));
  }, [sessionId]);

  useEffect(() => {
    if (!recording) {
      setElapsed(0);
      return;
    }
    const interval = setInterval(() => {
      setElapsed((e) => e + 1);
    }, 1000);
    return () => clearInterval(interval);
  }, [recording]);

  const handleToggle = useCallback(async () => {
    if (recording) {
      const id = await stopRecording(sessionId, cols, rows);
      setRecording(false);
      if (id) {
        window.dispatchEvent(
          new CustomEvent("recording-saved", { detail: { id } }),
        );
      }
    } else {
      startRecording(sessionId);
      setRecording(true);
    }
  }, [recording, sessionId, cols, rows]);

  const formatTime = (s: number) => {
    const m = Math.floor(s / 60);
    const sec = s % 60;
    return `${m.toString().padStart(2, "0")}:${sec.toString().padStart(2, "0")}`;
  };

  return (
    <button
      onClick={handleToggle}
      className={cn(
        "flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] transition-all",
        recording
          ? "bg-red-500/20 text-red-400 hover:bg-red-500/30"
          : "bg-muted/20 text-muted-foreground/50 hover:bg-muted/30 hover:text-muted-foreground/80",
      )}
      title={recording ? "Stop recording" : "Start recording"}
    >
      {recording ? (
        <>
          <Square className="w-2.5 h-2.5 fill-current" />
          <Clock className="w-2.5 h-2.5" />
          <span className="font-mono">{formatTime(elapsed)}</span>
        </>
      ) : (
        <>
          <Circle className="w-2.5 h-2.5 fill-red-500/60 text-red-500/60" />
          <span>REC</span>
        </>
      )}
    </button>
  );
});

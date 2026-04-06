import { useCallback, useEffect, useRef, useState } from "react";
import {
  Clock,
  Pause,
  Play,
  SkipBack,
  SkipForward,
  Trash2,
  X,
} from "lucide-react";
import { Terminal as XTerm } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { cn } from "@/lib/utils";
import { ThemeManager } from "@/lib/theme";
import {
  deleteRecording,
  listRecordings,
  loadRecording,
  type Recording,
  type RecordingMeta,
} from "@/lib/terminal/recording";
import { useStore } from "@/store";

export function RecordingsPanel({ onClose }: { onClose: () => void }) {
  const currentProjectPath = useStore((s) => s.currentProjectPath);
  const [recordings, setRecordings] = useState<RecordingMeta[]>([]);
  const [activeRecording, setActiveRecording] = useState<Recording | null>(null);
  const [playing, setPlaying] = useState(false);
  const [progress, setProgress] = useState(0);
  const [speed, setSpeed] = useState(1);
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<XTerm | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const eventIdxRef = useRef(0);

  const load = useCallback(async () => {
    const list = await listRecordings();
    setRecordings(list);
  }, []);

  useEffect(() => {
    load();
  }, [load, currentProjectPath]);

  useEffect(() => {
    if (!containerRef.current || !activeRecording) return;

    const term = new XTerm({
      cursorBlink: false,
      disableStdin: true,
      fontSize: 13,
      fontFamily: '"JetBrains Mono", "Fira Code", "Cascadia Code", Menlo, monospace',
      theme: ThemeManager.getCurrentTheme(),
      cols: activeRecording.meta.width || 80,
      rows: activeRecording.meta.height || 24,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(containerRef.current);
    try {
      fit.fit();
    } catch {
      /* ignore */
    }
    termRef.current = term;
    fitRef.current = fit;

    return () => {
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
    };
  }, [activeRecording]);

  const stopPlayback = useCallback(() => {
    if (timerRef.current) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
    setPlaying(false);
  }, []);

  const playFrom = useCallback(
    (idx: number) => {
      if (!activeRecording || !termRef.current) return;
      const events = activeRecording.events;
      if (idx >= events.length) {
        setPlaying(false);
        setProgress(100);
        return;
      }

      eventIdxRef.current = idx;
      const [elapsed, data] = events[idx];
      termRef.current.write(data);

      const pct =
        events.length > 0
          ? Math.round(((idx + 1) / events.length) * 100)
          : 100;
      setProgress(pct);

      if (idx + 1 < events.length) {
        const nextElapsed = events[idx + 1][0];
        const delay = Math.max(0, ((nextElapsed - elapsed) * 1000) / speed);
        timerRef.current = setTimeout(() => playFrom(idx + 1), delay);
      } else {
        setPlaying(false);
        setProgress(100);
      }
    },
    [activeRecording, speed],
  );

  const handlePlay = useCallback(() => {
    if (!activeRecording) return;
    if (playing) {
      stopPlayback();
      return;
    }
    setPlaying(true);
    if (progress >= 100) {
      termRef.current?.reset();
      setProgress(0);
      eventIdxRef.current = 0;
      playFrom(0);
    } else {
      playFrom(eventIdxRef.current);
    }
  }, [activeRecording, playing, progress, playFrom, stopPlayback]);

  const handleRestart = useCallback(() => {
    stopPlayback();
    termRef.current?.reset();
    setProgress(0);
    eventIdxRef.current = 0;
  }, [stopPlayback]);

  const handleOpen = useCallback(async (meta: RecordingMeta) => {
    stopPlayback();
    const rec = await loadRecording(meta.id);
    if (rec) {
      setActiveRecording(rec);
      setProgress(0);
      eventIdxRef.current = 0;
    }
  }, [stopPlayback]);

  const handleDelete = useCallback(
    async (id: string) => {
      if (!confirm("Delete this recording?")) return;
      await deleteRecording(id);
      if (activeRecording?.meta.id === id) {
        setActiveRecording(null);
        stopPlayback();
      }
      load();
    },
    [activeRecording, load, stopPlayback],
  );

  const formatDuration = (ms: number) => {
    const s = Math.floor(ms / 1000);
    const m = Math.floor(s / 60);
    const sec = s % 60;
    return `${m}:${sec.toString().padStart(2, "0")}`;
  };

  return (
    <div className="flex-1 flex flex-col h-full overflow-hidden bg-card rounded-xl">
      <div className="flex items-center justify-between px-4 py-2.5 border-b border-border/10 flex-shrink-0">
        <div className="flex items-center gap-2">
          <Clock className="w-4 h-4 text-accent/70" />
          <span className="text-sm font-medium">Terminal Recordings</span>
        </div>
        <button
          onClick={onClose}
          className="p-1 rounded hover:bg-muted/30 text-muted-foreground/50 hover:text-muted-foreground"
        >
          <X className="w-3.5 h-3.5" />
        </button>
      </div>

      <div className="flex-1 flex min-h-0">
        {/* List */}
        <div className="w-56 flex-shrink-0 border-r border-border/10 overflow-y-auto">
          {recordings.length === 0 ? (
            <div className="p-4 text-center text-[11px] text-muted-foreground/40">
              No recordings yet. Use the REC button in the terminal to start recording.
            </div>
          ) : (
            <div className="p-2 space-y-1">
              {recordings.map((rec) => (
                <div
                  key={rec.id}
                  className={cn(
                    "px-2.5 py-2 rounded-md cursor-pointer transition-colors group",
                    activeRecording?.meta.id === rec.id
                      ? "bg-accent/10 text-accent"
                      : "hover:bg-muted/20",
                  )}
                  onClick={() => handleOpen(rec)}
                >
                  <div className="flex items-center justify-between">
                    <span className="text-xs font-medium truncate">{rec.title}</span>
                    <button
                      className="p-0.5 rounded opacity-0 group-hover:opacity-100 hover:bg-red-500/20 text-muted-foreground/40 hover:text-red-400 transition-all"
                      onClick={(e) => {
                        e.stopPropagation();
                        handleDelete(rec.id);
                      }}
                    >
                      <Trash2 className="w-3 h-3" />
                    </button>
                  </div>
                  <div className="flex items-center gap-2 mt-0.5 text-[10px] text-muted-foreground/40">
                    <span>{formatDuration(rec.duration_ms)}</span>
                    <span>{rec.event_count} events</span>
                  </div>
                  <div className="text-[9px] text-muted-foreground/30 mt-0.5">
                    {new Date(rec.created_at).toLocaleDateString()}
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* Player */}
        <div className="flex-1 flex flex-col min-w-0">
          {activeRecording ? (
            <>
              <div className="flex items-center gap-2 px-3 py-2 border-b border-border/10">
                <button
                  onClick={handleRestart}
                  className="p-1 rounded hover:bg-muted/30 text-muted-foreground/50"
                  title="Restart"
                >
                  <SkipBack className="w-3.5 h-3.5" />
                </button>
                <button
                  onClick={handlePlay}
                  className={cn(
                    "p-1.5 rounded-md transition-colors",
                    playing
                      ? "bg-accent/20 text-accent"
                      : "bg-muted/20 text-muted-foreground/70 hover:bg-muted/30",
                  )}
                  title={playing ? "Pause" : "Play"}
                >
                  {playing ? (
                    <Pause className="w-4 h-4" />
                  ) : (
                    <Play className="w-4 h-4" />
                  )}
                </button>
                <div className="flex-1 h-1.5 rounded-full bg-muted/20 overflow-hidden mx-2">
                  <div
                    className="h-full rounded-full bg-accent/60 transition-all duration-200"
                    style={{ width: `${progress}%` }}
                  />
                </div>
                <div className="flex items-center gap-1">
                  <SkipForward className="w-3 h-3 text-muted-foreground/40" />
                  <select
                    value={speed}
                    onChange={(e) => setSpeed(Number(e.target.value))}
                    className="bg-transparent text-[10px] text-muted-foreground/60 border-none outline-none cursor-pointer"
                  >
                    <option value={0.5}>0.5x</option>
                    <option value={1}>1x</option>
                    <option value={2}>2x</option>
                    <option value={4}>4x</option>
                    <option value={8}>8x</option>
                  </select>
                </div>
              </div>
              <div
                ref={containerRef}
                className="flex-1 p-2 overflow-hidden"
              />
            </>
          ) : (
            <div className="flex-1 flex items-center justify-center">
              <div className="text-center">
                <Play className="w-8 h-8 text-muted-foreground/20 mx-auto mb-2" />
                <p className="text-sm text-muted-foreground/40">
                  Select a recording to play
                </p>
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

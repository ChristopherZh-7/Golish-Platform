import { Plus, Trash2 } from "lucide-react";
import { CustomSelect } from "@/components/ui/custom-select";
import type { VulnFeed } from "./types";

interface VulnFeedsConfigProps {
  feeds: VulnFeed[];
  showAddFeed: boolean;
  setShowAddFeed: (v: boolean) => void;
  newFeed: { name: string; feed_type: string; url: string };
  setNewFeed: (updater: (f: { name: string; feed_type: string; url: string }) => { name: string; feed_type: string; url: string }) => void;
  handleToggleFeed: (id: string, enabled: boolean) => void;
  handleDeleteFeed: (id: string) => void;
  handleAddFeed: () => void;
}

export function VulnFeedsConfig({
  feeds,
  showAddFeed,
  setShowAddFeed,
  newFeed,
  setNewFeed,
  handleToggleFeed,
  handleDeleteFeed,
  handleAddFeed,
}: VulnFeedsConfigProps) {
  return (
    <div className="flex-1 overflow-y-auto px-3 py-2 space-y-1">
      {(feeds ?? []).map((feed) => (
        <div
          key={feed.id}
          className="flex items-center gap-2 py-1.5 px-2 rounded hover:bg-muted/5 group"
        >
          <input
            type="checkbox"
            checked={feed.enabled}
            onChange={() => handleToggleFeed(feed.id, !feed.enabled)}
            className="w-3 h-3 accent-accent"
          />
          <div className="flex-1 min-w-0">
            <div className="text-[10px] font-medium truncate">{feed.name}</div>
            <div className="text-[9px] text-muted-foreground/30 truncate">{feed.url}</div>
            {feed.last_fetched && (
              <div className="text-[8px] text-muted-foreground/20">
                Last fetched: {new Date(feed.last_fetched * 1000).toLocaleString()}
              </div>
            )}
          </div>
          <span className="text-[8px] text-muted-foreground/25 px-1.5 py-0.5 bg-muted/10 rounded">
            {feed.feed_type}
          </span>
          <button
            type="button"
            onClick={() => handleDeleteFeed(feed.id)}
            className="p-1 text-muted-foreground/20 hover:text-red-400 opacity-0 group-hover:opacity-100 transition-all"
          >
            <Trash2 className="w-3 h-3" />
          </button>
        </div>
      ))}

      {showAddFeed ? (
        <div className="space-y-1.5 p-2 border border-border/20 rounded">
          <input
            value={newFeed.name}
            onChange={(e) => setNewFeed((f) => ({ ...f, name: e.target.value }))}
            placeholder="Feed name..."
            className="w-full text-[10px] px-2 py-1 bg-background border border-border/30 rounded outline-none"
          />
          <CustomSelect
            value={newFeed.feed_type}
            onChange={(v) => setNewFeed((f) => ({ ...f, feed_type: v }))}
            options={[
              { value: "rss", label: "RSS / Atom Feed" },
              { value: "nvd", label: "NVD API" },
              { value: "cisa_kev", label: "CISA KEV" },
              { value: "custom", label: "Custom JSON" },
            ]}
            size="sm"
          />
          <input
            value={newFeed.url}
            onChange={(e) => setNewFeed((f) => ({ ...f, url: e.target.value }))}
            placeholder="Feed URL..."
            className="w-full text-[10px] px-2 py-1 bg-background border border-border/30 rounded outline-none"
          />
          <div className="flex gap-1.5">
            <button
              type="button"
              onClick={handleAddFeed}
              disabled={!newFeed.name.trim() || !newFeed.url.trim()}
              className="text-[9px] text-accent hover:text-accent/80 font-medium disabled:opacity-30"
            >
              Add
            </button>
            <button
              type="button"
              onClick={() => setShowAddFeed(false)}
              className="text-[9px] text-muted-foreground/30"
            >
              Cancel
            </button>
          </div>
        </div>
      ) : (
        <button
          type="button"
          onClick={() => setShowAddFeed(true)}
          className="flex items-center gap-1 text-[9px] text-muted-foreground/30 hover:text-accent transition-colors"
        >
          <Plus className="w-3 h-3" />
          Add feed
        </button>
      )}
    </div>
  );
}

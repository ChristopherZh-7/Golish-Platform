-- Persist interleaved reasoning ("thinking") bursts so restored history keeps
-- multiple Thought blocks in time order instead of collapsing to one. Mirrors
-- the tool_call_offsets column: a nullable JSONB array of ThinkingSegment.
ALTER TABLE chat_messages ADD COLUMN IF NOT EXISTS thinking_segments JSONB;

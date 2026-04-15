ALTER TABLE chat_messages ADD COLUMN IF NOT EXISTS tool_call_offsets JSONB;

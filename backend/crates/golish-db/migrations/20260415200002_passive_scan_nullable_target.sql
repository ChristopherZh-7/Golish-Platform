-- Allow passive_scan_logs to have NULL target_id for ZAP auto-synced records
-- where the scanned URL might not match an existing target entry.
ALTER TABLE passive_scan_logs ALTER COLUMN target_id DROP NOT NULL;

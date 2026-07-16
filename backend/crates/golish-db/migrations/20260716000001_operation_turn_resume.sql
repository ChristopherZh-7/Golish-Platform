-- Durable top-level Turn history for one stable Task/Operation Thread.
-- A continuation closes the previous open Turn and appends a new ordinal;
-- it never creates a replacement operation or rewrites worker chains.

CREATE TABLE operation_turns (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE CASCADE,
    ordinal BIGINT NOT NULL CHECK (ordinal > 0),
    trigger_input TEXT NOT NULL,
    status TEXT NOT NULL CHECK (
        status IN ('running', 'waiting', 'completed', 'interrupted', 'failed')
    ),
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    terminal_at TIMESTAMPTZ,
    UNIQUE (operation_id, ordinal),
    CHECK (
        (status IN ('running', 'waiting') AND terminal_at IS NULL)
        OR
        (status IN ('completed', 'interrupted', 'failed') AND terminal_at IS NOT NULL)
    )
);

CREATE UNIQUE INDEX operation_turns_one_open
    ON operation_turns(operation_id)
    WHERE status IN ('running', 'waiting');

CREATE INDEX operation_turns_timeline
    ON operation_turns(operation_id, ordinal DESC);

INSERT INTO operation_turns(
    id, operation_id, ordinal, trigger_input, status, started_at, terminal_at
)
SELECT
    uuid_generate_v4(),
    task.id,
    1,
    task.input,
    CASE task.status::TEXT
        WHEN 'waiting' THEN 'waiting'
        WHEN 'finished' THEN 'completed'
        WHEN 'failed' THEN 'failed'
        ELSE 'running'
    END,
    task.created_at,
    CASE
        WHEN task.status::TEXT IN ('finished', 'failed') THEN task.updated_at
        ELSE NULL
    END
FROM tasks AS task
JOIN operation_state AS operation ON operation.operation_id=task.id
ON CONFLICT (operation_id, ordinal) DO NOTHING;

CREATE FUNCTION sync_open_operation_turn_from_task_status()
RETURNS trigger AS $$
BEGIN
    IF NEW.status::TEXT = 'waiting' THEN
        UPDATE operation_turns
           SET status='waiting'
         WHERE operation_id=NEW.id AND status='running';
    ELSIF NEW.status::TEXT = 'finished' THEN
        UPDATE operation_turns
           SET status='completed', terminal_at=COALESCE(terminal_at, NOW())
         WHERE operation_id=NEW.id AND status IN ('running', 'waiting');
    ELSIF NEW.status::TEXT = 'failed' THEN
        UPDATE operation_turns
           SET status='failed', terminal_at=COALESCE(terminal_at, NOW())
         WHERE operation_id=NEW.id AND status IN ('running', 'waiting');
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER tasks_sync_open_operation_turn
AFTER UPDATE OF status ON tasks
FOR EACH ROW
WHEN (OLD.status IS DISTINCT FROM NEW.status)
EXECUTE FUNCTION sync_open_operation_turn_from_task_status();

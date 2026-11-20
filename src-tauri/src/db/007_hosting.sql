ALTER TABLE batch_runs ADD COLUMN mode TEXT NOT NULL DEFAULT 'automatic' CHECK (mode IN ('automatic','assisted'));
ALTER TABLE batch_steps ADD COLUMN problem TEXT NOT NULL DEFAULT '';
CREATE INDEX batch_chapter_activity ON batch_steps(batch_id, chapter_id, created_at);

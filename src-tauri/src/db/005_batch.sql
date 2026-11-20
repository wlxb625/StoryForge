CREATE TABLE batch_runs (
    batch_id TEXT PRIMARY KEY,
    chapters_json TEXT NOT NULL,
    cursor INTEGER NOT NULL DEFAULT 0,
    phase TEXT NOT NULL DEFAULT 'write',
    audited_revision_id TEXT REFERENCES chapter_revisions(revision_id),
    state TEXT NOT NULL DEFAULT 'running',
    budget_usd REAL NOT NULL,
    spent_usd REAL NOT NULL DEFAULT 0,
    instruction TEXT NOT NULL DEFAULT '',
    writer_profile TEXT NOT NULL,
    audit_profile TEXT NOT NULL,
    extraction_profile TEXT NOT NULL,
    reason TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL
);
CREATE UNIQUE INDEX one_live_batch ON batch_runs((1)) WHERE state IN('running','paused');
CREATE TABLE batch_steps (
    task_id TEXT PRIMARY KEY REFERENCES generation_tasks(task_id),
    batch_id TEXT NOT NULL REFERENCES batch_runs(batch_id),
    chapter_id TEXT NOT NULL REFERENCES chapters(chapter_id),
    phase TEXT NOT NULL,
    reserved_usd REAL NOT NULL,
    settled_usd REAL,
    adopted_revision_id TEXT REFERENCES chapter_revisions(revision_id),
    created_at TEXT NOT NULL
);
CREATE UNIQUE INDEX one_pending_batch_step ON batch_steps(batch_id) WHERE settled_usd IS NULL;

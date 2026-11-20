ALTER TABLE chapters ADD COLUMN indexed_revision_id TEXT REFERENCES chapter_revisions(revision_id);
ALTER TABLE project_meta ADD COLUMN audit_profile_id TEXT;
ALTER TABLE project_meta ADD COLUMN extraction_profile_id TEXT;
ALTER TABLE character_knowledge ADD COLUMN perception TEXT NOT NULL DEFAULT '';
ALTER TABLE story_events ADD COLUMN active_revision_id TEXT REFERENCES chapter_revisions(revision_id);
UPDATE story_events SET active_revision_id=source_revision_id;

DROP TABLE chapters_fts;
CREATE VIRTUAL TABLE chapters_fts USING fts5(
    chapter_id UNINDEXED,
    bigram_text,
    content='',
    contentless_delete=1,
    tokenize='unicode61'
);
CREATE UNIQUE INDEX idx_knowledge_target ON character_knowledge(character_id, target_event_id);
CREATE INDEX idx_review_candidates ON story_events(approval_status, created_at);
CREATE INDEX idx_revision_task ON generation_tasks(base_revision_id, task_type, state);

CREATE TABLE memory_ingest (
    task_id TEXT PRIMARY KEY REFERENCES generation_tasks(task_id),
    problem TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL
);

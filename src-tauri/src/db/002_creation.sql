CREATE TABLE writing_scenes (
    scene_id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    scene_json TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1,
    updated_at TEXT NOT NULL
);
ALTER TABLE chapters ADD COLUMN active_scene_id TEXT REFERENCES writing_scenes(scene_id);

CREATE TABLE prompt_revisions (
    prompt_revision_id TEXT PRIMARY KEY,
    instruction TEXT NOT NULL,
    created_at TEXT NOT NULL
);

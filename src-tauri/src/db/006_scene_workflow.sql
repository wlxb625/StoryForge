ALTER TABLE chapters ADD COLUMN scene_ids_json TEXT NOT NULL DEFAULT '[]';
UPDATE chapters SET scene_ids_json=json_array(active_scene_id) WHERE active_scene_id IS NOT NULL;
ALTER TABLE chapter_revisions ADD COLUMN scene_id TEXT REFERENCES writing_scenes(scene_id);
ALTER TABLE batch_runs ADD COLUMN draft_task_id TEXT REFERENCES generation_tasks(task_id);
ALTER TABLE soft_notes ADD COLUMN remind_after_chapters INTEGER;

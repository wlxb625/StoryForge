ALTER TABLE story_events ADD COLUMN active_anchor_json TEXT;
ALTER TABLE stage_arcs ADD COLUMN version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE stage_arcs ADD COLUMN pivot_start_order INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stage_arcs ADD COLUMN writer_visible INTEGER NOT NULL DEFAULT 0;
CREATE TABLE arc_pivots (
    pivot_id TEXT PRIMARY KEY,
    arc_id TEXT NOT NULL REFERENCES stage_arcs(arc_id),
    previous_goal TEXT NOT NULL,
    next_goal TEXT NOT NULL,
    instruction TEXT NOT NULL,
    future_chapters INTEGER NOT NULL,
    affected_notes_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

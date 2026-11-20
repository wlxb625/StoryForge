use crate::{
    db::memory::{
        Audience, Candidate, Canon, IndexInput, Knowledge, Memory, MemoryRoles, Note, SearchHit,
    },
    error::Result,
    Studio,
};
use tauri::State;

#[tauri::command]
pub fn read_memory(studio: State<'_, Studio>) -> Result<Memory> {
    studio.project(|project| project.memory())
}

#[tauri::command]
pub fn write_canon(studio: State<'_, Studio>, entry: Canon) -> Result<Canon> {
    studio.project(|project| project.save_canon(entry))
}

#[tauri::command]
pub fn write_note(studio: State<'_, Studio>, note: Note) -> Result<()> {
    studio.project(|project| project.save_note(note))
}

#[tauri::command]
pub fn write_knowledge(studio: State<'_, Studio>, knowledge: Knowledge) -> Result<()> {
    studio.project(|project| project.save_knowledge(knowledge))
}

#[tauri::command]
pub fn write_memory_roles(studio: State<'_, Studio>, roles: MemoryRoles) -> Result<()> {
    studio.project(|project| project.save_memory_roles(roles))
}

#[tauri::command]
pub fn ingest_memory(
    studio: State<'_, Studio>,
    task_id: String,
    candidates: Vec<Candidate>,
    problem: String,
) -> Result<()> {
    studio.project(|project| project.ingest_memory(&task_id, candidates, problem))
}

#[tauri::command]
pub fn review_memory_fact(
    studio: State<'_, Studio>,
    event_id: String,
    action: String,
    audience: Option<Audience>,
) -> Result<()> {
    studio.project(|project| project.review_fact(&event_id, &action, audience))
}

#[tauri::command]
pub fn refresh_memory_status(
    studio: State<'_, Studio>,
    chapter_id: String,
    revision_id: String,
) -> Result<()> {
    studio.project(|project| project.memory_refresh(&chapter_id, &revision_id))
}

#[tauri::command]
pub fn stale_indexes(studio: State<'_, Studio>) -> Result<Vec<IndexInput>> {
    studio.project(|project| project.stale_indexes())
}

#[tauri::command]
pub fn index_chapter(
    studio: State<'_, Studio>,
    chapter_id: String,
    revision_id: String,
    bigrams: String,
) -> Result<bool> {
    studio.project(|project| project.index_chapter(&chapter_id, &revision_id, &bigrams))
}

#[tauri::command]
pub fn search_text(
    studio: State<'_, Studio>,
    phrase: String,
    exact: String,
    after_order: Option<i64>,
) -> Result<Vec<SearchHit>> {
    studio.project(|project| project.search_text(&phrase, &exact, after_order))
}

use crate::{
    db::{
        editing::{Arc, Pivot, PivotPreview},
        Revision,
    },
    error::Result,
    Studio,
};
use tauri::State;

#[tauri::command]
pub fn list_arcs(studio: State<'_, Studio>) -> Result<Vec<Arc>> {
    studio.project(|project| project.arcs())
}

#[tauri::command]
pub fn write_arc(studio: State<'_, Studio>, arc: Arc) -> Result<Arc> {
    studio.project(|project| project.write_arc(arc))
}

#[tauri::command]
pub fn assign_arc(
    studio: State<'_, Studio>,
    chapter_id: String,
    arc_id: Option<String>,
) -> Result<()> {
    studio.project(|project| project.assign_arc(&chapter_id, arc_id.as_deref()))
}

#[tauri::command]
pub fn preview_pivot(studio: State<'_, Studio>, arc_id: String) -> Result<PivotPreview> {
    studio.project(|project| project.preview_pivot(&arc_id))
}

#[tauri::command]
pub fn apply_pivot(studio: State<'_, Studio>, pivot: Pivot) -> Result<Arc> {
    studio.project(|project| project.apply_pivot(pivot))
}

#[tauri::command]
pub fn apply_polish(
    studio: State<'_, Studio>,
    task_id: String,
    canon_confirmed: bool,
) -> Result<Revision> {
    studio.project(|project| project.apply_polish(&task_id, canon_confirmed))
}

use crate::{
    db::catalog::{Catalog, Pack, PromptRevision, Scene, SceneInput, Trace},
    error::Result,
    Studio,
};
use tauri::State;

#[tauri::command]
pub fn style_catalog(studio: State<'_, Studio>) -> Result<Catalog> {
    studio.project(|project| project.catalog())
}
#[tauri::command]
pub fn write_style_packs(
    studio: State<'_, Studio>,
    packs: Vec<Pack>,
    seed: bool,
) -> Result<Catalog> {
    studio.project(|project| project.write_packs(packs, seed))
}
#[tauri::command]
pub fn enable_style_pack(
    studio: State<'_, Studio>,
    item_id: String,
    enabled: bool,
) -> Result<Catalog> {
    studio.project(|project| project.set_pack_enabled(&item_id, enabled))
}
#[tauri::command]
pub fn choose_style(
    studio: State<'_, Studio>,
    base: Option<String>,
    cartridge: Option<String>,
) -> Result<Catalog> {
    studio.project(|project| project.active_style(base.as_deref(), cartridge.as_deref()))
}
#[tauri::command]
pub fn list_scenes(studio: State<'_, Studio>) -> Result<Vec<Scene>> {
    studio.project(|project| project.scenes())
}
#[tauri::command]
pub fn save_scene(studio: State<'_, Studio>, input: SceneInput) -> Result<Scene> {
    studio.project(|project| project.save_scene(input))
}
#[tauri::command]
pub fn link_scene(
    studio: State<'_, Studio>,
    chapter_id: String,
    scene_id: Option<String>,
) -> Result<()> {
    studio.project(|project| project.link_scene(&chapter_id, scene_id.as_deref()))
}
#[tauri::command]
pub fn prompt_revisions(studio: State<'_, Studio>) -> Result<Vec<PromptRevision>> {
    studio.project(|project| project.prompt_revisions())
}
#[tauri::command]
pub fn save_prompt_revision(
    studio: State<'_, Studio>,
    instruction: String,
) -> Result<PromptRevision> {
    studio.project(|project| project.save_prompt_revision(&instruction))
}
#[tauri::command]
pub fn list_llm_traces(studio: State<'_, Studio>) -> Result<Vec<Trace>> {
    studio.project(|project| project.traces())
}

use crate::{
    db::{
        batch::{profile_price, Batch, BatchActivity, BatchInput, BatchSummary},
        Revision,
    },
    error::{Result, StudioError},
    guard, Studio,
};
use tauri::State;

#[tauri::command]
pub fn latest_batch(studio: State<'_, Studio>) -> Result<Option<Batch>> {
    studio.project(|project| project.latest_batch())
}

#[tauri::command]
pub fn batch_history(studio: State<'_, Studio>) -> Result<Vec<BatchSummary>> {
    studio.project(|project| project.batch_history())
}

#[tauri::command]
pub fn named_batch(studio: State<'_, Studio>, batch_id: String) -> Result<Batch> {
    studio.project(|project| project.named_batch(&batch_id))
}

#[tauri::command]
pub fn batch_activity(
    studio: State<'_, Studio>,
    batch_id: String,
    chapter_id: String,
) -> Result<Vec<BatchActivity>> {
    studio.project(|project| project.batch_activity(&batch_id, &chapter_id))
}

#[tauri::command]
pub fn start_batch(studio: State<'_, Studio>, input: BatchInput) -> Result<Batch> {
    let profiles = guard(&studio.profiles)?;
    for id in [
        &input.writer_profile,
        &input.audit_profile,
        &input.extraction_profile,
    ] {
        let profile = profiles.get(id)?;
        profile.credential_for_request()?;
        if profile_price(&profile).is_none() {
            return Err(StudioError::new(
                400,
                "主笔、审查和提取档案都需要配置每百万 token 单价",
            ));
        }
    }
    drop(profiles);
    studio.project(|project| project.start_batch(input))
}

#[tauri::command]
pub fn control_batch(
    studio: State<'_, Studio>,
    batch_id: String,
    action: String,
    budget_usd: Option<f64>,
    reason: Option<String>,
) -> Result<Batch> {
    studio
        .project(|project| project.control_batch(&batch_id, &action, budget_usd, reason.as_deref()))
}

#[tauri::command]
pub fn reserve_batch_step(
    studio: State<'_, Studio>,
    batch_id: String,
    task_id: String,
    profile_id: String,
) -> Result<Batch> {
    let profile = guard(&studio.profiles)?.get(&profile_id)?;
    studio.project(|project| project.reserve_batch_step(&batch_id, &task_id, &profile))
}

#[tauri::command]
pub fn adopt_batch_draft(
    studio: State<'_, Studio>,
    batch_id: String,
    task_id: String,
) -> Result<Revision> {
    studio.project(|project| project.adopt_batch_draft(&batch_id, &task_id))
}

#[tauri::command]
pub fn preview_batch_draft(
    studio: State<'_, Studio>,
    batch_id: String,
) -> Result<Option<crate::db::batch::BatchDraft>> {
    studio.project(|project| project.preview_batch_draft(&batch_id))
}

#[tauri::command]
pub fn settle_batch_step(
    studio: State<'_, Studio>,
    batch_id: String,
    task_id: String,
    problem: String,
    passed: bool,
) -> Result<Batch> {
    studio.project(|project| project.settle_batch_step(&batch_id, &task_id, &problem, passed))
}

#[tauri::command]
pub fn advance_batch(studio: State<'_, Studio>, batch_id: String) -> Result<Batch> {
    studio.project(|project| project.advance_batch(&batch_id))
}

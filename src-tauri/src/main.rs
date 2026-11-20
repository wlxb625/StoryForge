#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod batch_commands;
mod catalog_commands;
mod db;
mod editing_commands;
mod error;
mod exports;
mod memory_commands;
mod profiles;
mod remote;

use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard},
    time::Duration,
};

use db::{Chapter, Project, ProjectInfo, Revision, RevisionInput, Task, TaskEvent, TaskInput};
use error::{Result, StudioError};
use futures_util::future::{AbortHandle, Abortable};
use profiles::{Capabilities, ListedProfile, Profile, ProfileInput, ProfileStore};
use remote::{Outcome, Remote};
use serde::Serialize;
use serde_json::Value;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State, WindowEvent,
};
use tauri_plugin_dialog::DialogExt;

const EVENTS: &str = "storyforge-task";

struct Studio {
    project: Mutex<Option<Project>>,
    streams: Mutex<HashMap<String, AbortHandle>>,
    profiles: Mutex<ProfileStore>,
    remote: Remote,
}

fn guard<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>> {
    mutex
        .lock()
        .map_err(|_| StudioError::new(500, "本机存储暂时不可用"))
}

impl Studio {
    fn project<T>(&self, action: impl FnOnce(&mut Project) -> Result<T>) -> Result<T> {
        let mut opened = guard(&self.project)?;
        action(
            opened
                .as_mut()
                .ok_or_else(|| StudioError::new(409, "请先打开一个小说工程"))?,
        )
    }
}

fn publish(app: &AppHandle, event: &TaskEvent) {
    let _ = app.emit(EVENTS, event);
}

fn swap_project(app: &AppHandle, load: impl FnOnce() -> Result<Project>) -> Result<ProjectInfo> {
    let studio = app.state::<Studio>();
    let streams = guard(&studio.streams)?;
    if !streams.is_empty() {
        return Err(StudioError::new(
            409,
            "续写任务正在运行，请先结束任务再切换工程",
        ));
    }
    let mut active = guard(&studio.project)?;
    if let Some(current) = active.as_ref() {
        if current
            .latest_batch()?
            .is_some_and(|batch| batch.state == "running")
        {
            return Err(StudioError::new(
                409,
                "后台批次正在运行，请先暂停再切换工程",
            ));
        }
    }
    let mut project = load()?;
    project.recover()?;
    let info = project.info.clone();
    *active = Some(project);
    Ok(info)
}

#[tauri::command]
async fn new_project(app: AppHandle, title: String) -> Result<Option<ProjectInfo>> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut destination = match app
            .dialog()
            .file()
            .set_title("新建小说工程")
            .add_filter("StoryForge 工程", &["story"])
            .set_file_name("我的小说.story")
            .blocking_save_file()
        {
            Some(file) => file
                .into_path()
                .map_err(|_| StudioError::new(400, "请选择本机文件"))?,
            None => return Ok(None),
        };
        if destination.extension().is_none() {
            destination.set_extension("story");
        }
        swap_project(&app, || Project::create(destination, &title)).map(Some)
    })
    .await
    .map_err(|_| StudioError::new(500, "无法新建工程"))?
}

#[tauri::command]
async fn open_project(app: AppHandle) -> Result<Option<ProjectInfo>> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = match app
            .dialog()
            .file()
            .set_title("打开小说工程")
            .add_filter("StoryForge 工程", &["story"])
            .blocking_pick_file()
        {
            Some(file) => file
                .into_path()
                .map_err(|_| StudioError::new(400, "请选择本机文件"))?,
            None => return Ok(None),
        };
        swap_project(&app, || Project::open(path)).map(Some)
    })
    .await
    .map_err(|_| StudioError::new(500, "无法打开工程"))?
}

#[tauri::command]
fn rename_project(studio: State<'_, Studio>, title: String) -> Result<ProjectInfo> {
    studio.project(|project| project.rename_project(&title))
}

#[tauri::command]
fn close_project(studio: State<'_, Studio>) -> Result<()> {
    let streams = guard(&studio.streams)?;
    if !streams.is_empty() {
        return Err(StudioError::new(409, "续写任务仍在运行"));
    }
    let mut project = guard(&studio.project)?;
    if let Some(current) = project.as_ref() {
        if current
            .latest_batch()?
            .is_some_and(|batch| batch.state == "running")
        {
            return Err(StudioError::new(409, "请先暂停后台批次再关闭工程"));
        }
    }
    *project = None;
    Ok(())
}

#[tauri::command]
fn current_project(studio: State<'_, Studio>) -> Result<Option<ProjectInfo>> {
    Ok(guard(&studio.project)?
        .as_ref()
        .map(|project| project.info.clone()))
}

#[tauri::command]
fn list_chapters(studio: State<'_, Studio>) -> Result<Vec<Chapter>> {
    studio.project(|project| project.chapters())
}

#[tauri::command]
fn archived_chapters(studio: State<'_, Studio>) -> Result<Vec<Chapter>> {
    studio.project(|project| project.archived_chapters())
}

#[tauri::command]
fn restore_chapter(studio: State<'_, Studio>, chapter_id: String) -> Result<()> {
    studio.project(|project| project.restore_chapter(&chapter_id))
}

#[tauri::command]
fn insert_chapter(
    studio: State<'_, Studio>,
    title: String,
    after: Option<String>,
) -> Result<Chapter> {
    studio.project(|project| project.insert_chapter(&title, after.as_deref()))
}

#[tauri::command]
fn rename_chapter(studio: State<'_, Studio>, chapter_id: String, title: String) -> Result<()> {
    studio.project(|project| project.rename_chapter(&chapter_id, &title))
}

#[tauri::command]
fn move_chapter(studio: State<'_, Studio>, chapter_id: String, direction: i64) -> Result<()> {
    studio.project(|project| project.move_chapter(&chapter_id, direction))
}

#[tauri::command]
fn archive_chapter(studio: State<'_, Studio>, chapter_id: String) -> Result<()> {
    studio.project(|project| project.archive_chapter(&chapter_id))
}

#[tauri::command]
fn list_revisions(studio: State<'_, Studio>, chapter_id: String) -> Result<Vec<Revision>> {
    studio.project(|project| project.revisions(&chapter_id))
}

#[tauri::command]
fn save_revision(studio: State<'_, Studio>, input: RevisionInput) -> Result<Revision> {
    studio.project(|project| project.save_revision(input))
}

#[tauri::command]
fn save_canon_revision(
    studio: State<'_, Studio>,
    input: RevisionInput,
    canon: db::memory::Canon,
) -> Result<Revision> {
    studio.project(|project| project.save_revision_with_canon(input, Some(canon)))
}

#[tauri::command]
fn list_model_profiles(studio: State<'_, Studio>) -> Result<Vec<ListedProfile>> {
    guard(&studio.profiles)?.list()
}

#[tauri::command]
fn save_model_profile(studio: State<'_, Studio>, input: ProfileInput) -> Result<Profile> {
    guard(&studio.profiles)?.upsert(input)
}

#[tauri::command]
fn remove_model_profile(studio: State<'_, Studio>, profile_id: String) -> Result<()> {
    guard(&studio.profiles)?.remove(&profile_id)
}

#[tauri::command]
async fn keyring_set_secret(app: AppHandle, profile_id: String) -> Result<bool> {
    tauri::async_runtime::spawn_blocking(move || {
        let studio = app.state::<Studio>();
        let store = guard(&studio.profiles)?;
        profiles::prompt_secret(&store.get(&profile_id)?)
    })
    .await
    .map_err(|_| StudioError::new(500, "无法读取系统凭据窗口"))?
}

#[tauri::command]
fn keyring_delete_secret(studio: State<'_, Studio>, profile_id: String) -> Result<()> {
    guard(&studio.profiles)?.get(&profile_id)?.delete_secret()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProbeResult {
    #[serde(flatten)]
    outcome: Outcome,
    content: String,
    has_cached_tokens: bool,
}

#[tauri::command]
async fn model_probe(
    studio: State<'_, Studio>,
    profile_id: String,
    body: Value,
) -> Result<ProbeResult> {
    if !remote::probe_body_valid(&body) {
        return Err(StudioError::new(400, "探测请求格式有误"));
    }
    let profile = guard(&studio.profiles)?.get(&profile_id)?;
    let secret = profile.credential_for_request()?;
    let mut content = String::new();
    let (outcome, usage) = studio
        .remote
        .call(
            &profile,
            secret.as_deref(),
            &body,
            Duration::from_secs(35),
            |text| {
                content.push_str(text);
                Ok(())
            },
        )
        .await;
    Ok(ProbeResult {
        outcome,
        content,
        has_cached_tokens: usage.cached_tokens.is_some(),
    })
}

#[tauri::command]
fn save_capabilities(
    studio: State<'_, Studio>,
    profile_id: String,
    profile_version: String,
    capabilities: Capabilities,
) -> Result<Capabilities> {
    let store = guard(&studio.profiles)?;
    let profile = store.get(&profile_id)?;
    if profile.version != profile_version {
        return Err(StudioError::new(409, "模型档案已修改，请重新探测"));
    }
    store.update_capabilities(&profile, capabilities)
}

#[tauri::command]
fn create_generation_task(studio: State<'_, Studio>, input: TaskInput) -> Result<Task> {
    studio.project(|project| project.create_task(input))
}

#[tauri::command]
fn list_generation_tasks(
    studio: State<'_, Studio>,
    chapter_id: Option<String>,
) -> Result<Vec<Task>> {
    studio.project(|project| project.tasks(chapter_id.as_deref()))
}

#[tauri::command]
fn generation_task(studio: State<'_, Studio>, task_id: String) -> Result<Task> {
    studio.project(|project| project.task(&task_id))
}

#[tauri::command]
fn task_events(
    studio: State<'_, Studio>,
    task_id: String,
    from_event_seq: i64,
) -> Result<Vec<TaskEvent>> {
    studio.project(|project| project.events(&task_id, from_event_seq))
}

#[tauri::command]
fn llm_stream_start(app: AppHandle, studio: State<'_, Studio>, task_id: String) -> Result<bool> {
    let mut streams = guard(&studio.streams)?;
    if streams.contains_key(&task_id) {
        return Ok(false);
    }
    let task = studio.project(|project| {
        let task = project.task(&task_id)?;
        if task.state == "queued" {
            project.check_writer_stamp(&task)?;
            project.check_batch_dispatch(&task)?;
        }
        Ok(task)
    })?;
    if task.state != "queued" {
        return Ok(false);
    }
    let context = &task.frozen_context;
    let profile_id = context["profileId"]
        .as_str()
        .ok_or_else(|| StudioError::new(400, "任务缺少模型档案标识"))?;
    let profile = guard(&studio.profiles)?.get(profile_id)?;
    if context["profileVersion"].as_str() != Some(profile.version.as_str()) {
        return Err(StudioError::new(
            409,
            "模型档案有变化，请从当前版本新建任务",
        ));
    }
    let secret = profile.credential_for_request()?;
    let body = context["requestBody"].clone();
    if !body["messages"].is_array()
        || body["messages"][0]["content"] != context["prompt"]
        || body["model"].as_str() != Some(profile.model.as_str())
    {
        return Err(StudioError::new(400, "请求内容与冻结快照不一致"));
    }
    let Some(event) = studio.project(|project| project.claim_task(&task_id))? else {
        return Ok(false);
    };
    let (abort, registration) = AbortHandle::new_pair();
    streams.insert(task_id.clone(), abort);
    publish(&app, &event);
    tauri::async_runtime::spawn(async move {
        let studio = app.state::<Studio>();
        let work = studio.remote.call(
            &profile,
            secret.as_deref(),
            &body,
            Duration::from_secs(300),
            |text| {
                if let Some(event) =
                    studio.project(|project| project.append_token(&task_id, text))?
                {
                    publish(&app, &event);
                }
                Ok(())
            },
        );
        if let Ok((outcome, usage)) = Abortable::new(work, registration).await {
            if let Ok(Some(event)) = studio.project(|project| {
                project.finish_task(
                    &task_id,
                    outcome.state,
                    outcome.message.as_deref(),
                    outcome.finish_reason.as_deref(),
                    Some(usage),
                )
            }) {
                publish(&app, &event);
            }
        }
        if let Ok(mut active) = guard(&studio.streams) {
            active.remove(&task_id);
        };
    });
    Ok(true)
}

#[tauri::command]
fn llm_stream_cancel(
    app: AppHandle,
    studio: State<'_, Studio>,
    task_id: String,
    technical_issue: Option<String>,
) -> Result<bool> {
    let mut streams = guard(&studio.streams)?;
    let state = if technical_issue.is_some() {
        "failed"
    } else {
        "cancelled"
    };
    let message = technical_issue
        .as_deref()
        .unwrap_or("已停止续写，收到的正文已保留。");
    let event = studio
        .project(|project| project.finish_task(&task_id, state, Some(message), None, None))?;
    if let Some(abort) = streams.remove(&task_id) {
        abort.abort();
    }
    if let Some(event) = event {
        publish(&app, &event);
        Ok(true)
    } else {
        Ok(false)
    }
}

fn foreground(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            foreground(app)
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_sql::Builder::default().build())
        .setup(|app| {
            let executable = std::env::current_exe()?;
            let portable = executable
                .parent()
                .filter(|path| path.join("portable.flag").is_file());
            let settings = match portable {
                Some(path) => path.join("settings"),
                None => app.path().app_config_dir()?,
            };
            let profile_file = settings.join("model-profiles.json");
            let remote = Remote::new().map_err(|error| std::io::Error::other(error.message))?;
            app.manage(Studio {
                project: Mutex::new(None),
                streams: Mutex::new(HashMap::new()),
                profiles: Mutex::new(ProfileStore {
                    config_file: profile_file,
                }),
                remote,
            });
            let show = MenuItem::with_id(app, "show", "显示写作台", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出 StoryForge", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            let mut tray = TrayIconBuilder::with_id("storyforge")
                .tooltip("StoryForge 写作台")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => foreground(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if matches!(
                        event,
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        }
                    ) {
                        foreground(tray.app_handle());
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            catalog_commands::style_catalog,
            catalog_commands::write_style_packs,
            catalog_commands::enable_style_pack,
            catalog_commands::choose_style,
            catalog_commands::list_scenes,
            catalog_commands::save_scene,
            catalog_commands::link_scene,
            catalog_commands::prompt_revisions,
            catalog_commands::save_prompt_revision,
            catalog_commands::list_llm_traces,
            memory_commands::read_memory,
            memory_commands::write_canon,
            memory_commands::write_note,
            memory_commands::write_knowledge,
            memory_commands::write_memory_roles,
            memory_commands::ingest_memory,
            memory_commands::review_memory_fact,
            memory_commands::refresh_memory_status,
            memory_commands::stale_indexes,
            memory_commands::index_chapter,
            memory_commands::search_text,
            editing_commands::list_arcs,
            editing_commands::write_arc,
            editing_commands::assign_arc,
            editing_commands::preview_pivot,
            editing_commands::apply_pivot,
            editing_commands::apply_polish,
            exports::export_project,
            exports::export_pack,
            batch_commands::latest_batch,
            batch_commands::batch_history,
            batch_commands::named_batch,
            batch_commands::batch_activity,
            batch_commands::start_batch,
            batch_commands::control_batch,
            batch_commands::reserve_batch_step,
            batch_commands::adopt_batch_draft,
            batch_commands::preview_batch_draft,
            batch_commands::settle_batch_step,
            batch_commands::advance_batch,
            new_project,
            open_project,
            rename_project,
            close_project,
            current_project,
            list_chapters,
            archived_chapters,
            restore_chapter,
            insert_chapter,
            rename_chapter,
            move_chapter,
            archive_chapter,
            list_revisions,
            save_revision,
            save_canon_revision,
            list_model_profiles,
            save_model_profile,
            remove_model_profile,
            keyring_set_secret,
            keyring_delete_secret,
            model_probe,
            save_capabilities,
            create_generation_task,
            list_generation_tasks,
            generation_task,
            task_events,
            llm_stream_start,
            llm_stream_cancel,
        ])
        .run(tauri::generate_context!())
        .expect("无法启动 StoryForge");
}

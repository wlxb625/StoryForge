import { invoke, isTauri } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type {
  Capabilities, Chapter, GenerationTask, ModelProfile, ProbeResult, ProfileInput, Project, Revision,
  RevisionInput, TaskEvent, TaskInput, Pack, StyleCatalog, Scene, SceneInput, PromptRevision, Trace,
} from '../contracts/types'

const call = <T>(name: string, args?: Record<string, unknown>) => {
  if (!isTauri()) return Promise.reject(new Error('请在 StoryForge 桌面应用中打开工程。'))
  return invoke<T>(name, args)
}

export const desktop = {
  newProject: (title: string) => call<Project | null>('new_project', { title }),
  openProject: () => call<Project | null>('open_project'),
  renameProject: (title: string) => call<Project>('rename_project', { title }),
  closeProject: () => call<void>('close_project'),
  currentProject: () => call<Project | null>('current_project'),
  chapters: () => call<Chapter[]>('list_chapters'),
  archivedChapters: () => call<Chapter[]>('archived_chapters'),
  restoreChapter: (chapterId: string) => call<void>('restore_chapter', { chapterId }),
  insertChapter: (title: string, after?: string) => call<Chapter>('insert_chapter', { title, after: after ?? null }),
  renameChapter: (chapterId: string, title: string) => call<void>('rename_chapter', { chapterId, title }),
  moveChapter: (chapterId: string, direction: -1 | 1) => call<void>('move_chapter', { chapterId, direction }),
  archiveChapter: (chapterId: string) => call<void>('archive_chapter', { chapterId }),
  revisions: (chapterId: string) => call<Revision[]>('list_revisions', { chapterId }),
  saveRevision: (input: RevisionInput) => call<Revision>('save_revision', { input }),
  profiles: () => call<ModelProfile[]>('list_model_profiles'),
  saveProfile: (input: ProfileInput) => call<ModelProfile>('save_model_profile', { input }),
  removeProfile: (profileId: string) => call<void>('remove_model_profile', { profileId }),
  setSecret: (profileId: string) => call<boolean>('keyring_set_secret', { profileId }),
  deleteSecret: (profileId: string) => call<void>('keyring_delete_secret', { profileId }),
  probe: (profileId: string, body: Record<string, unknown>) => call<ProbeResult>('model_probe', { profileId, body }),
  saveCapabilities: (profile: ModelProfile, capabilities: Capabilities) =>
    call<Capabilities>('save_capabilities', { profileId: profile.id, profileVersion: profile.version, capabilities }),
  createTask: (input: TaskInput) => call<GenerationTask>('create_generation_task', { input }),
  tasks: (chapterId?: string) => call<GenerationTask[]>('list_generation_tasks', { chapterId: chapterId ?? null }),
  task: (taskId: string) => call<GenerationTask>('generation_task', { taskId }),
  events: (taskId: string, fromEventSeq: number) => call<TaskEvent[]>('task_events', { taskId, fromEventSeq }),
  start: (taskId: string) => call<boolean>('llm_stream_start', { taskId }),
  cancel: (taskId: string, technicalIssue?: string) => call<boolean>('llm_stream_cancel', { taskId, technicalIssue: technicalIssue ?? null }),
  catalog: () => call<StyleCatalog>('style_catalog'),
  writePacks: (packs: Pack[], seed = false) => call<StyleCatalog>('write_style_packs', { packs, seed }),
  enablePack: (itemId: string, enabled: boolean) => call<StyleCatalog>('enable_style_pack', { itemId, enabled }),
  chooseStyle: (base: string | null, cartridge: string | null) => call<StyleCatalog>('choose_style', { base, cartridge }),
  scenes: () => call<Scene[]>('list_scenes'),
  saveScene: (input: SceneInput) => call<Scene>('save_scene', { input }),
  linkScene: (chapterId: string, sceneId: string | null) => call<void>('link_scene', { chapterId, sceneId }),
  prompts: () => call<PromptRevision[]>('prompt_revisions'),
  savePrompt: (instruction: string) => call<PromptRevision>('save_prompt_revision', { instruction }),
  traces: () => call<Trace[]>('list_llm_traces'),
  listen: (onEvent: (event: TaskEvent) => void) =>
    listen<TaskEvent>('storyforge-task', ({ payload }) => onEvent(payload)),
}

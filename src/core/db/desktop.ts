import { invoke, isTauri } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type { Audience, Candidate, Canon, Knowledge, Memory, MemoryRoles, Note, SearchHit } from '../narrative/memory'
import type { Pivot, PivotPreview, StageArc } from '../narrative/editing'
import type { Batch, BatchInput } from '../tasks/batch'
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
  memory: () => call<Memory>('read_memory'),
  writeCanon: (entry: Canon) => call<Canon>('write_canon', { entry }),
  writeNote: (note: Note) => call<void>('write_note', { note }),
  writeKnowledge: (knowledge: Knowledge) => call<void>('write_knowledge', { knowledge }),
  writeMemoryRoles: (roles: MemoryRoles) => call<void>('write_memory_roles', { roles }),
  ingestMemory: (taskId: string, candidates: Candidate[], problem: string) => call<void>('ingest_memory', { taskId, candidates, problem }),
  reviewFact: (eventId: string, action: 'accept' | 'reject' | 'undo', audience?: Audience) =>
    call<void>('review_memory_fact', { eventId, action, audience: audience ?? null }),
  refreshMemoryStatus: (chapterId: string, revisionId: string) => call<void>('refresh_memory_status', { chapterId, revisionId }),
  staleIndexes: () => call<Array<{ chapterId: string; revisionId: string; content: string }>>('stale_indexes'),
  indexChapter: (chapterId: string, revisionId: string, bigrams: string) => call<boolean>('index_chapter', { chapterId, revisionId, bigrams }),
  searchText: (phrase: string, exact: string, afterOrder?: number) => call<SearchHit[]>('search_text', { phrase, exact, afterOrder: afterOrder ?? null }),
  arcs: () => call<StageArc[]>('list_arcs'),
  writeArc: (arc: StageArc) => call<StageArc>('write_arc', { arc }),
  assignArc: (chapterId: string, arcId: string | null) => call<void>('assign_arc', { chapterId, arcId }),
  previewPivot: (arcId: string) => call<PivotPreview>('preview_pivot', { arcId }),
  applyPivot: (pivot: Pivot) => call<StageArc>('apply_pivot', { pivot }),
  applyPolish: (taskId: string, canonConfirmed: boolean) => call<Revision>('apply_polish', { taskId, canonConfirmed }),
  exportProject: (format: 'story' | 'docx' | 'epub') => call<string | null>('export_project', { format }),
  latestBatch: () => call<Batch | null>('latest_batch'),
  startBatch: (input: BatchInput) => call<Batch>('start_batch', { input }),
  controlBatch: (batchId: string, action: 'pause' | 'resume' | 'stop', budgetUsd?: number, reason?: string) =>
    call<Batch>('control_batch', { batchId, action, budgetUsd: budgetUsd ?? null, reason: reason ?? null }),
  reserveBatchStep: (batchId: string, taskId: string, profileId: string) => call<Batch>('reserve_batch_step', { batchId, taskId, profileId }),
  adoptBatchDraft: (batchId: string, taskId: string) => call<Revision>('adopt_batch_draft', { batchId, taskId }),
  settleBatchStep: (batchId: string, taskId: string, problem: string, passed: boolean) =>
    call<Batch>('settle_batch_step', { batchId, taskId, problem, passed }),
  advanceBatch: (batchId: string) => call<Batch>('advance_batch', { batchId }),
  listen: (onEvent: (event: TaskEvent) => void) =>
    listen<TaskEvent>('storyforge-task', ({ payload }) => onEvent(payload)),
}

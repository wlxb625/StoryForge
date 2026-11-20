export type TaskState =
  | 'created' | 'queued' | 'running' | 'streaming' | 'completed'
  | 'failed' | 'cancelled' | 'interrupted' | 'outcome_unknown'

export type Verification = 'verified_supported' | 'declared' | 'unknown' | 'verified_unsupported'

export interface Capabilities {
  streaming?: Verification | null
  jsonMode?: Verification | null
  usage?: Verification | null
  cancel?: Verification | null
  contextLen?: Verification | null
  promptCache?: Verification | null
  multimodal?: Verification | null
}

export interface ModelProfile {
  id: string
  version: string
  name: string
  model: string
  baseUrl: string
  needsKey: boolean
  hasKey: boolean
  direct: boolean
  tokenLimitField: 'max_tokens' | 'max_completion_tokens'
  capabilities: Capabilities
  rates?: Rates | null
}

export type ProfileInput = Pick<ModelProfile, 'name' | 'model' | 'baseUrl' | 'needsKey' | 'direct' | 'tokenLimitField'> & {
  id?: string
  rates?: Rates | null
}

export interface Rates { input: number; output: number; cacheRead?: number | null; cacheWrite?: number | null }

export interface ProbeResult {
  state: TaskState
  status: number | null
  streamed: boolean
  content: string
  usageObserved: boolean
  hasCachedTokens: boolean
  message: string | null
}

export interface Project {
  projectId: string
  title: string
  path: string
  targetBeatChars: number
  targetChapterChars: number
}

export interface Chapter {
  chapterId: string
  chapterOrder: number
  title: string
  currentRevisionId: string | null
  status: string
  activeSceneId?: string | null
  memorySyncStatus?: 'pending' | 'processing' | 'synced' | 'needs_confirm'
}

export interface Revision {
  revisionId: string
  parentRevisionId: string | null
  saveKind: 'formal_revision' | 'ai_draft' | 'auto_buffer'
  sourceTag: string
  content: string
  cjkCharCount: number
  createdAt: string
}

export interface RevisionInput {
  chapterId: string
  baseRevisionId: string | null
  content: string
  source: 'manual_edit' | 'ai_gen' | 'interrupted_draft' | 'auto_buffer'
  draftTaskId?: string
}

export interface FrozenContext {
  schemaVersion: 1
  capturedAt: string
  projectId: string
  baseRevisionId: string | null
  previousRevisionIds: string[]
  chapterTitle: string
  canonVersions: Record<string, number>
  memoryStamp?: string
  skillVersions: Record<string, string>
  targetBeatChars: number
  instruction: string
  carryDraft: string
  profileId: string
  profileVersion: string
  model: string
  prompt: string
  requestBody: Record<string, unknown>
  scene?: Scene
  intent?: SceneIntent
  promptRevisionId?: string | null
  activeSkills?: Array<{ id: string; name: string; version: string }>
  comparison?: { id: string; label: 'A' | 'B' }
  rates?: Rates | null
}

export interface TaskInput {
  idempotencyKey: string
  chapterId: string
  baseRevisionId: string | null
  taskType: 'beat_write' | 'ab_write' | 'skill_extract' | 'memory_extract' | 'consistency_check'
  frozenContext: FrozenContext
}

export interface GenerationTask extends Omit<TaskInput, 'idempotencyKey'> {
  taskId: string
  state: TaskState
  partialOutput: string
  lastEventSeq: number
}

export type SkillAxis = 'narrative' | 'character' | 'emotion' | 'dialogue' | 'humor' | 'pacing'

export interface Pack {
  itemId: string
  itemKind: 'writing_skill' | 'genre_cartridge'
  slot: 'base_style' | 'technique' | 'cartridge'
  name: string
  version: string
  priority: number
  metadata: {
    axis?: SkillAxis
    triggers?: string[]
    antiTriggers?: string[]
    avoidPhrases?: string[]
    description?: string
    stages?: string[]
    preferredSkills?: string[]
  }
  body: string
  enabled: boolean
}

export interface StyleCatalog {
  packs: Pack[]
  activeBaseSkillId: string | null
  activePackId: string | null
}

export interface SceneIntent {
  goal: string
  hiddenGoal: string
  action: string
  emotion: string
  dialogue: string
  pace: string
  humor: '允许' | '克制' | '禁止'
  pov: string
}

export interface SceneDetails {
  time: string
  location: string
  cast: string
  conflict: string
  intent?: Partial<SceneIntent>
  techniqueIds?: string[]
  povCharacterId?: string
}
export interface Scene { sceneId: string; title: string; version: number; details: SceneDetails }
export type SceneInput = Partial<Pick<Scene, 'sceneId' | 'version'>> & Pick<Scene, 'title' | 'details'>
export interface PromptRevision { promptRevisionId: string; instruction: string; createdAt: string }
export interface Trace {
  traceId: string
  taskId: string
  modelProfileId: string
  stage: string
  promptTokens: number | null
  cachedTokens: number | null
  completionTokens: number | null
  costUsd: number | null
  latencyMs: number
  status: string
  createdAt: string
}

export interface TaskEvent {
  taskId: string
  eventSeq: number
  eventType: 'status' | 'token' | 'completed' | 'error' | 'outcome_unknown'
  payload: {
    state?: TaskState
    text?: string
    message?: string | null
    finishReason?: string | null
  }
}

export function errorMessage(error: unknown): string {
  if (typeof error === 'string') return error
  if (error && typeof error === 'object' && 'message' in error) return String(error.message)
  return '操作未能完成。请重试。'
}

export function activeTask(state: TaskState): boolean {
  return state === 'queued' || state === 'running' || state === 'streaming'
}

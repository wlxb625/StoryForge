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
}

export type ProfileInput = Pick<ModelProfile, 'name' | 'model' | 'baseUrl' | 'needsKey' | 'direct' | 'tokenLimitField'> & {
  id?: string
}

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
  skillVersions: Record<string, string>
  targetBeatChars: number
  instruction: string
  carryDraft: string
  profileId: string
  profileVersion: string
  model: string
  prompt: string
  requestBody: Record<string, unknown>
}

export interface TaskInput {
  idempotencyKey: string
  chapterId: string
  baseRevisionId: string | null
  taskType: 'beat_write'
  frozenContext: FrozenContext
}

export interface GenerationTask extends Omit<TaskInput, 'idempotencyKey' | 'taskType'> {
  taskId: string
  state: TaskState
  partialOutput: string
  lastEventSeq: number
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

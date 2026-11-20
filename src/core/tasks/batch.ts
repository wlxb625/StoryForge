export interface BatchInput {
  chapterIds: string[]
  budgetUsd: number
  instruction: string
  writerProfile: string
  auditProfile: string
  extractionProfile: string
  mode?: 'automatic' | 'assisted'
}
export interface Batch extends BatchInput {
  batchId: string
  cursor: number
  phase: 'write' | 'audit' | 'approval' | 'adopt' | 'extract' | 'advance'
  draftTaskId: string | null
  auditedRevisionId: string | null
  state: 'running' | 'paused' | 'stopped' | 'completed'
  mode: 'automatic' | 'assisted'
  spentUsd: number
  reason: string
  estimatedSteps: number
  step: { taskId: string; reservedUsd: number; adoptedRevisionId: string | null } | null
}
export interface BatchDraft { taskId: string; content: string; appendix: string }
export interface BatchSummary { batchId: string; state: Batch['state']; mode: Batch['mode']; createdAt: string }
export interface BatchActivity {
  taskId: string
  phase: 'write' | 'audit' | 'extract'
  state: string
  model: string
  createdAt: string
  costUsd: number | null
  revisionId: string | null
  problem: string
}
export type BatchAction = 'pause' | 'resume' | 'approve' | 'stop' | 'reaudit' | 'reset_draft'

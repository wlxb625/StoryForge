export interface BatchInput {
  chapterIds: string[]
  budgetUsd: number
  instruction: string
  writerProfile: string
  auditProfile: string
  extractionProfile: string
}
export interface Batch extends BatchInput {
  batchId: string
  cursor: number
  phase: 'write' | 'audit' | 'extract' | 'advance'
  auditedRevisionId: string | null
  state: 'running' | 'paused' | 'stopped' | 'completed'
  spentUsd: number
  reason: string
  estimatedSteps: number
  step: { taskId: string; reservedUsd: number; adoptedRevisionId: string | null } | null
}

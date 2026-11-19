import { describe, expect, it, vi } from 'vitest'
import type { GenerationTask, TaskEvent, TaskInput } from '../contracts/types'
import { TaskService } from './service'

function fixture() {
  const input: TaskInput = {
    idempotencyKey: 'single-click', chapterId: 'chapter', baseRevisionId: null, taskType: 'beat_write',
    frozenContext: {
      schemaVersion: 1, capturedAt: '2026-11-19T00:00:00Z', projectId: 'project', baseRevisionId: null,
      previousRevisionIds: [], chapterTitle: '起点', canonVersions: {}, skillVersions: {}, targetBeatChars: 900,
      instruction: '', carryDraft: '', model: 'local', profileId: 'profile', profileVersion: 'version',
      prompt: '续写', requestBody: { model: 'local' },
    },
  }
  const task: GenerationTask = {
    taskId: 'task', chapterId: input.chapterId, baseRevisionId: input.baseRevisionId,
    frozenContext: input.frozenContext, partialOutput: '', lastEventSeq: 1, state: 'queued', taskType: 'beat_write',
  }
  return { input, task }
}

describe('TaskService', () => {
  it('coalesces a double-click and rejects different content under the same key', async () => {
    const { input, task } = fixture()
    const start = vi.fn(async () => true)
    const createTask = vi.fn(async () => task)
    const api = { createTask, start, cancel: vi.fn(), events: vi.fn(), listen: vi.fn() }
    const service = new TaskService(api)
    const first = service.createTask(input)
    const duplicate = service.createTask(input)
    expect(duplicate).toBe(first)
    await expect(service.createTask({ ...input, chapterId: 'different' })).rejects.toMatchObject({ status: 409 })
    expect(await first).toEqual(task)
    expect(createTask).toHaveBeenCalledTimes(1)
  })

  it('replays a sequence gap once and ignores duplicate notifications', async () => {
    const { task } = fixture()
    const seen: number[] = []
    const stop = vi.fn()
    const events: TaskEvent[] = [
      { taskId: task.taskId, eventSeq: 2, eventType: 'token', payload: { state: 'streaming', text: '她推' } },
      { taskId: task.taskId, eventSeq: 3, eventType: 'token', payload: { state: 'streaming', text: '开门' } },
    ]
    let notify: ((event: TaskEvent) => void) | undefined
    let replayEnabled = false
    const api = {
      createTask: vi.fn(), start: vi.fn(), cancel: vi.fn(),
      events: vi.fn(async (_id: string, from: number) => replayEnabled ? events.filter((evt) => evt.eventSeq > from) : []),
      listen: vi.fn(async (callback: (event: TaskEvent) => void) => { notify = callback; return stop }),
    }
    const failure = vi.fn()
    const dispose = new TaskService(api).subscribeEvents(task.taskId, 1, (event) => seen.push(event.eventSeq), failure)
    await vi.waitFor(() => expect(api.events).toHaveBeenCalled())
    replayEnabled = true
    notify?.(events[1]!)
    notify?.(events[0]!)
    await vi.waitFor(() => expect(seen).toEqual([2, 3]))
    expect(failure).not.toHaveBeenCalled()
    dispose()
    expect(stop).toHaveBeenCalledOnce()
  })
})

import type { GenerationTask, TaskEvent, TaskInput } from '../contracts/types'
import { desktop } from '../db/desktop'

type TaskTransport = Pick<typeof desktop, 'createTask' | 'start' | 'cancel' | 'events' | 'listen'>

export class TaskService {
  private readonly creating = new Map<string, { fingerprint: string; promise: Promise<GenerationTask> }>()

  constructor(private readonly remote: TaskTransport = desktop) {}

  createTask(input: TaskInput): Promise<GenerationTask> {
    const fingerprint = JSON.stringify(input)
    const prior = this.creating.get(input.idempotencyKey)
    if (prior) {
      return prior.fingerprint === fingerprint
        ? prior.promise
        : Promise.reject({ status: 409, message: '同一请求标识不能用于不同内容。' })
    }
    const promise = this.remote.createTask(input).finally(() => this.creating.delete(input.idempotencyKey))
    this.creating.set(input.idempotencyKey, { fingerprint, promise })
    return promise
  }

  startTask(taskId: string) { return this.remote.start(taskId) }
  cancelTask(taskId: string, issue?: string) { return this.remote.cancel(taskId, issue) }

  subscribeEvents(taskId: string, fromEventSeq: number, onEvent: (event: TaskEvent) => void, onFailure: (error: unknown) => void) {
    let cursor = fromEventSeq
    let stopped = false
    let unlisten: (() => void) | undefined
    let chain = Promise.resolve()

    const apply = (event: TaskEvent) => {
      if (stopped || event.eventSeq <= cursor) return
      cursor = event.eventSeq
      onEvent(event)
    }
    const drain = async () => {
      while (!stopped) {
        const replay = await this.remote.events(taskId, cursor)
        for (const event of replay) apply(event)
        if (replay.length < 1000) break
      }
    }
    const enqueue = (action: () => Promise<void>) => {
      chain = chain.then(action).catch((error) => { if (!stopped) onFailure(error) })
    }
    void this.remote.listen((event) => {
      if (event.taskId !== taskId || stopped) return
      enqueue(async () => {
        if (event.eventSeq > cursor + 1) await drain()
        if (event.eventSeq === cursor + 1) apply(event)
      })
    }).then((stop) => {
      if (stopped) stop()
      else {
        unlisten = stop
        enqueue(drain)
      }
    }).catch(onFailure)

    return () => {
      stopped = true
      unlisten?.()
    }
  }
}

export const taskService = new TaskService()

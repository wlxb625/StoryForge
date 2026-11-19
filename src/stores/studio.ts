import { defineStore } from 'pinia'
import { computed, ref, watch } from 'vue'
import {
  activeTask, errorMessage, type Chapter, type GenerationTask, type ModelProfile,
  type Project, type Revision, type TaskEvent,
} from '../core/contracts/types'
import { desktop } from '../core/db/desktop'
import { freezeContext } from '../core/narrative/freeze'
import { countHan, technicalIssue } from '../core/quality/hardGate'
import { taskService } from '../core/tasks/service'

export const useStudio = defineStore('studio', () => {
  const project = ref<Project | null>(null)
  const chapters = ref<Chapter[]>([])
  const archived = ref<Chapter[]>([])
  const chapterId = ref('')
  const revisions = ref<Revision[]>([])
  const profiles = ref<ModelProfile[]>([])
  const profileId = ref('')
  const tasks = ref<GenerationTask[]>([])
  const taskId = ref('')
  const text = ref('')
  const acceptedText = ref('')
  const draftHint = ref('')
  const error = ref('')
  const notice = ref('')
  const busy = ref(false)
  const writingIssues = ref<Record<string, string>>({})
  const subscriptions = new Map<string, () => void>()
  let bufferTimer: ReturnType<typeof setTimeout> | undefined
  let noticeTimer: ReturnType<typeof setTimeout> | undefined

  const chapter = computed(() => chapters.value.find((entry) => entry.chapterId === chapterId.value))
  const selectedProfile = computed(() => profiles.value.find((entry) => entry.id === profileId.value))
  const chapterTasks = computed(() => tasks.value.filter((entry) => entry.chapterId === chapterId.value))
  const currentTask = computed(() => chapterTasks.value.find((entry) => entry.taskId === taskId.value) ?? chapterTasks.value[0])
  const currentActive = computed(() => chapterTasks.value.find((entry) => activeTask(entry.state)))
  const currentCount = computed(() => countHan(text.value))
  const dirty = computed(() => text.value !== acceptedText.value)
  const history = computed(() => revisions.value.filter((revision) => revision.saveKind !== 'auto_buffer'))
  const queuedCount = computed(() => tasks.value.filter((task) => activeTask(task.state)).length)

  function flash(message: string) {
    notice.value = message
    clearTimeout(noticeTimer)
    noticeTimer = setTimeout(() => { notice.value = '' }, 2400)
  }

  async function run(operation: () => Promise<void>) {
    if (busy.value) return
    busy.value = true
    error.value = ''
    try {
      await operation()
    } catch (problem) {
      error.value = errorMessage(problem)
    } finally {
      busy.value = false
    }
  }

  async function readProfiles() {
    profiles.value = await desktop.profiles()
    if (!profiles.value.some((entry) => entry.id === profileId.value)) {
      profileId.value = profiles.value.find((entry) => !entry.needsKey || entry.hasKey)?.id ?? profiles.value[0]?.id ?? ''
    }
  }

  async function readChapters() {
    const [current, saved] = await Promise.all([desktop.chapters(), desktop.archivedChapters()])
    chapters.value = current
    archived.value = saved
  }

  function resetSubscriptions() {
    for (const stop of subscriptions.values()) stop()
    subscriptions.clear()
  }

  async function loadWorkspace(result: Project) {
    resetSubscriptions()
    project.value = result
    await readChapters()
    tasks.value = await desktop.tasks()
    writingIssues.value = {}
    for (const task of tasks.value.filter((item) => activeTask(item.state))) watchTask(task)
    await readChapter(chapters.value[0]?.chapterId ?? '')
  }

  function openWorkspace(title?: string) {
    return run(async () => {
      await persistBuffer()
      const result = title === undefined ? await desktop.openProject() : await desktop.newProject(title)
      if (result) await loadWorkspace(result)
    })
  }

  function initialize() {
    return run(async () => {
      await readProfiles()
      const opened = await desktop.currentProject()
      if (opened) await loadWorkspace(opened)
    })
  }

  function closeWorkspace() {
    return run(async () => {
      await persistBuffer()
      await desktop.closeProject()
      resetSubscriptions()
      project.value = null
      chapterId.value = ''
      tasks.value = []
    })
  }

  function renameWorkspace(title: string) {
    return run(async () => { project.value = await desktop.renameProject(title) })
  }

  async function readChapter(id: string) {
    clearTimeout(bufferTimer)
    chapterId.value = id
    taskId.value = ''
    draftHint.value = ''
    const selected = chapters.value.find((entry) => entry.chapterId === id)
    revisions.value = selected ? await desktop.revisions(id) : []
    acceptedText.value = revisions.value.find((entry) => entry.revisionId === selected?.currentRevisionId)?.content ?? ''
    const buffer = revisions.value.find((entry) => entry.saveKind === 'auto_buffer' && entry.parentRevisionId === selected?.currentRevisionId)
    text.value = buffer?.content ?? acceptedText.value
  }

  function openChapter(id: string) {
    if (id === chapterId.value) return
    return run(async () => {
      await persistBuffer()
      await readChapter(id)
    })
  }

  function addChapter(title: string, after?: string) {
    return run(async () => {
      await persistBuffer()
      const created = await desktop.insertChapter(title, after)
      await readChapters()
      await readChapter(created.chapterId)
    })
  }

  function renameChapter(title: string) {
    if (!chapter.value) return
    const id = chapter.value.chapterId
    return run(async () => {
      await desktop.renameChapter(id, title)
      await readChapters()
    })
  }

  function moveChapter(direction: -1 | 1) {
    if (!chapter.value) return
    const id = chapter.value.chapterId
    return run(async () => {
      await desktop.moveChapter(id, direction)
      await readChapters()
    })
  }

  function archiveSelected() {
    if (!chapter.value) return
    const id = chapter.value.chapterId
    return run(async () => {
      await persistBuffer()
      await desktop.archiveChapter(id)
      await readChapters()
      await readChapter(chapters.value[0]?.chapterId ?? '')
      flash('章节已归档')
    })
  }

  function restoreChapter(id: string) {
    return run(async () => {
      await persistBuffer()
      await desktop.restoreChapter(id)
      await readChapters()
      await readChapter(id)
    })
  }

  async function persistBuffer() {
    clearTimeout(bufferTimer)
    const entry = chapter.value
    if (!project.value || !entry || !dirty.value) return
    await desktop.saveRevision({
      chapterId: entry.chapterId, baseRevisionId: entry.currentRevisionId, source: 'auto_buffer', content: text.value,
    })
  }

  watch(text, () => {
    clearTimeout(bufferTimer)
    if (!dirty.value || busy.value) return
    bufferTimer = setTimeout(() => {
      void persistBuffer().catch((problem) => { error.value = errorMessage(problem) })
    }, 850)
  })

  async function saveText() {
    clearTimeout(bufferTimer)
    const entry = chapter.value
    if (!entry || !dirty.value) return
    const revision = await desktop.saveRevision({
      chapterId: entry.chapterId, baseRevisionId: entry.currentRevisionId,
      source: 'manual_edit', content: text.value,
    })
    acceptedText.value = revision.content
    entry.currentRevisionId = revision.revisionId
    entry.status = 'accepted'
    revisions.value.unshift(revision)
    flash('已保存正式修订')
  }

  function saveCurrent() { return run(saveText) }

  function restoreRevision(revision: Revision) {
    text.value = revision.content
    flash('已载入所选版本，可保存为新修订')
  }

  function onTaskEvent(event: TaskEvent) {
    const task = tasks.value.find((entry) => entry.taskId === event.taskId)
    if (!task || task.lastEventSeq >= event.eventSeq) return
    task.lastEventSeq = event.eventSeq
    if (event.payload.text) task.partialOutput += event.payload.text
    if (event.payload.state) task.state = event.payload.state
    if (event.payload.message) writingIssues.value[task.taskId] = event.payload.message
    if (event.eventType === 'token' || event.eventType === 'completed') {
      const issue = technicalIssue(task.partialOutput, event.eventType === 'completed', event.payload.finishReason)
      if (issue && !writingIssues.value[task.taskId]) {
        writingIssues.value[task.taskId] = issue
        void taskService.cancelTask(task.taskId, issue).catch((problem) => { error.value = errorMessage(problem) })
      }
    }
    if (!activeTask(task.state) && (task.state !== 'completed' || !writingIssues.value[task.taskId])) {
      subscriptions.get(task.taskId)?.()
      subscriptions.delete(task.taskId)
    }
  }

  function watchTask(task: GenerationTask) {
    if (subscriptions.has(task.taskId)) return
    subscriptions.set(task.taskId, taskService.subscribeEvents(task.taskId, task.lastEventSeq, onTaskEvent,
      (problem) => { error.value = errorMessage(problem) }))
  }

  async function createAndRun(carryDraft = '', instruction = draftHint.value) {
    const book = project.value
    const profile = selectedProfile.value
    const entry = chapter.value
    if (!entry || !book) throw new Error('请选择章节。')
    if (!profile || (profile.needsKey && !profile.hasKey)) throw new Error('请先配置一个可用的模型。')
    if (currentActive.value) throw new Error('该章已有待处理任务。')
    await saveText()
    const revision = revisions.value.find((item) => item.revisionId === entry.currentRevisionId)
    const frozenContext = freezeContext(book, entry, revision, profile, instruction, carryDraft)
    const task = await taskService.createTask({
      idempotencyKey: crypto.randomUUID(), chapterId: entry.chapterId,
      baseRevisionId: entry.currentRevisionId, taskType: 'beat_write', frozenContext,
    })
    tasks.value.unshift(task)
    taskId.value = task.taskId
    watchTask(task)
    await taskService.startTask(task.taskId)
  }

  function generate() { return run(() => createAndRun()) }

  function startQueued(task: GenerationTask) {
    return run(async () => {
      watchTask(task)
      await taskService.startTask(task.taskId)
    })
  }

  function cancel(task: GenerationTask) {
    return run(async () => { await taskService.cancelTask(task.taskId) })
  }

  function resumeTask(task: GenerationTask, action: 'keep_draft' | 'continue_from_draft' | 'rerun_new_task') {
    return run(async () => {
      if (!chapter.value || task.chapterId !== chapter.value.chapterId || task.baseRevisionId !== chapter.value.currentRevisionId)
        throw new Error('章节已有更新，草稿仍可在任务历史中查看和复制。')
      if (action === 'keep_draft') {
        const revision = await desktop.saveRevision({
          chapterId: task.chapterId, baseRevisionId: task.baseRevisionId,
          source: 'interrupted_draft', draftTaskId: task.taskId, content: '',
        })
        revisions.value.unshift(revision)
        flash('已保留独立草稿')
        return
      }
      if (dirty.value) throw new Error('请先保存或恢复当前编辑，再选择恢复方式。')
      const carry = action === 'continue_from_draft'
        ? [task.frozenContext.carryDraft, task.partialOutput].filter(Boolean).join('\n\n') : ''
      await createAndRun(carry, task.frozenContext.instruction)
    })
  }

  function acceptDraft(task: GenerationTask) {
    return run(async () => {
      if (dirty.value) throw new Error('当前正文有未保存的修改，请先保存或手动合并。')
      const events = await desktop.events(task.taskId, Math.max(0, task.lastEventSeq - 8))
      const end = events.findLast((event) => event.eventType === 'completed')
      const issue = technicalIssue(task.partialOutput, true, end?.payload.finishReason)
      if (issue || writingIssues.value[task.taskId]) throw new Error(issue ?? writingIssues.value[task.taskId])
      const revision = await desktop.saveRevision({
        chapterId: task.chapterId, baseRevisionId: task.baseRevisionId, source: 'ai_gen',
        draftTaskId: task.taskId, content: '',
      })
      await readChapters()
      await readChapter(task.chapterId)
      flash(`已采纳为正式修订 · ${revision.cjkCharCount} 字`)
    })
  }

  return {
    project, chapters, archived, chapter, chapterId, profiles, profileId, selectedProfile, revisions, history, text, acceptedText,
    draftHint, error, notice, busy, dirty, currentCount, tasks, chapterTasks, taskId, currentTask, currentActive,
    queuedCount, writingIssues, initialize, openWorkspace, closeWorkspace, renameWorkspace, readProfiles, openChapter, addChapter,
    renameChapter, moveChapter, archiveSelected, restoreChapter, saveCurrent, restoreRevision, generate, startQueued,
    cancel, resumeTask, acceptDraft, flash,
  }
})

import { defineStore } from 'pinia'
import { computed, ref, watch } from 'vue'
import {
  activeTask, errorMessage, type Chapter, type GenerationTask, type ModelProfile,
  type Project, type Revision, type TaskEvent, type Pack, type Scene, type SceneInput, type StyleCatalog,
  type PromptRevision, type Trace, type FrozenContext, type TaskInput,
} from '../core/contracts/types'
import { desktop } from '../core/db/desktop'
import { freezeContext, retarget } from '../core/narrative/freeze'
import { activeStyles, deriveIntent } from '../core/narrative/intent'
import { countHan, technicalIssue } from '../core/quality/hardGate'
import { taskService } from '../core/tasks/service'
import type { Batch, BatchAction, BatchInput } from '../core/tasks/batch'
import { builtins } from '../core/skills/builtins'
import { skillExtraction } from '../core/skills/extraction'
import { cjkIndex, cjkQuery } from '../core/db/fts'
import { auditFindings, emptyMemory, extractCandidates, memoryAnalysis, writerMemory,
  type Audience, type Canon, type CanonImpact, type Fact, type Knowledge, type MemoryRoles, type Note, type SearchHit } from '../core/narrative/memory'
import { arcDirection, changeWindow, criticContext, makeSpan, parseReplacement, polishContext,
  type EditLevel, type Pivot, type StageArc } from '../core/narrative/editing'
import { sceneRanges, type SceneReview } from '../core/narrative/scenes'

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
  const autoFacts = ref<string[]>([])
  const busy = ref(false)
  const writingIssues = ref<Record<string, string>>({})
  const styles = ref<StyleCatalog>({ packs: [], activeBaseSkillId: null, activePackId: null })
  const scenes = ref<Scene[]>([])
  const promptHistory = ref<PromptRevision[]>([])
  const traces = ref<Trace[]>([])
  const memory = ref(emptyMemory())
  const canonProposal = ref<{ entry: Canon; impact: CanonImpact } | null>(null)
  const arcs = ref<StageArc[]>([])
  const batch = ref<Batch | null>(null)
  let pumping = false
  let pumpRequested = false
  const editImpact = ref<{ revisionId: string; hits: SearchHit[]; terms: string[] }>()
  let indexing: Promise<void> | undefined
  const subscriptions = new Map<string, () => void>()
  let bufferTimer: ReturnType<typeof setTimeout> | undefined
  let noticeTimer: ReturnType<typeof setTimeout> | undefined

  const chapter = computed(() => chapters.value.find((entry) => entry.chapterId === chapterId.value))
  const selectedProfile = computed(() => profiles.value.find((entry) => entry.id === profileId.value))
  const chapterTasks = computed(() => tasks.value.filter((entry) => entry.chapterId === chapterId.value && entry.taskType === 'beat_write'))
  const currentTask = computed(() => chapterTasks.value.find((entry) => entry.taskId === taskId.value) ?? chapterTasks.value[0])
  const currentActive = computed(() => chapterTasks.value.find((entry) => activeTask(entry.state)))
  const inspiration = computed(() => tasks.value.find(task => task.chapterId === chapterId.value && task.taskType === 'inspiration'
    && task.baseRevisionId === chapter.value?.currentRevisionId
    && (task.frozenContext.sceneBindingId ?? null) === (chapter.value?.activeSceneId ?? null)))
  const currentCount = computed(() => countHan(text.value))
  const dirty = computed(() => text.value !== acceptedText.value)
  const history = computed(() => revisions.value.filter((revision) => revision.saveKind !== 'auto_buffer'))
  const queuedCount = computed(() => tasks.value.filter((task) => activeTask(task.state)).length)
  const scene = computed(() => scenes.value.find((item) => item.sceneId === chapter.value?.activeSceneId))
  const sceneIntent = computed(() => deriveIntent(scene.value, draftHint.value, acceptedText.value,
    chapter.value ? arcDirection(arcs.value.find(arc => arc.arcId === chapter.value?.arcId), chapter.value) : ''))
  const sceneStyles = computed(() => activeStyles(styles.value, scene.value, sceneIntent.value, draftHint.value))
  const memoryPending = computed(() => memory.value.events.filter(event => event.approval === 'pending' && event.current).length)
  const chapterAnalysis = computed(() => tasks.value.filter(task => task.chapterId === chapterId.value
    && task.baseRevisionId === chapter.value?.currentRevisionId && !task.frozenContext.stagedDraftId
    && ['memory_extract', 'consistency_check'].includes(task.taskType)))
  const chapterAudits = computed(() => chapterAnalysis.value.filter(task => task.taskType === 'consistency_check'))
  const criticTasks = computed(() => tasks.value.filter(task => task.chapterId === chapterId.value &&
    task.baseRevisionId === chapter.value?.currentRevisionId && task.taskType === 'critic'))
  const polishTasks = computed(() => tasks.value.filter(task => task.chapterId === chapterId.value && task.taskType === 'polish_span'))
  const chapterFindings = computed(() => {
    const seen = new Set<string>()
    return chapterAudits.value.flatMap(task => {
      if (task.state !== 'completed') return []
      const scope = task.frozenContext.sceneReview
      const key = scope?.sceneId ?? ''
      if (seen.has(key)) return []
      seen.add(key)
      const local = task.frozenContext.viewpointAlerts ?? []
      try { return [...local, ...auditFindings(task.partialOutput, acceptedText.value, scope?.ranges)] }
      catch { return local }
    })
  })
  const criticFindings = computed(() => {
    const last = criticTasks.value.find(task => task.state === 'completed')
    const span = last?.frozenContext.span
    try { return last ? auditFindings(last.partialOutput, acceptedText.value, span ? [{ start: span.start, end: span.end }] : undefined) : [] }
    catch { return [] }
  })

  function flash(message: string) {
    autoFacts.value = []
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
    const [catalog, savedScenes, prompts, storedTasks, savedMemory, savedArcs, savedBatch] = await Promise.all([
      desktop.writePacks(builtins, true), desktop.scenes(), desktop.prompts(), desktop.tasks(), desktop.memory(), desktop.arcs(), desktop.latestBatch(), readChapters(),
    ])
    styles.value = catalog
    scenes.value = savedScenes
    promptHistory.value = prompts
    tasks.value = storedTasks
    writingIssues.value = {}
    memory.value = savedMemory
    canonProposal.value = null
    arcs.value = savedArcs
    batch.value = savedBatch
    editImpact.value = undefined
    for (const task of tasks.value.filter((item) => activeTask(item.state))) watchTask(task)
    const currentRevisions = new Set(chapters.value.map(item => item.currentRevisionId))
    for (const task of storedTasks) {
      if (task.state === 'queued' && task.frozenContext.batchId && task.taskId !== savedBatch?.step?.taskId) {
        await taskService.cancelTask(task.taskId)
        continue
      }
      const unprocessedExtraction = task.taskType === 'memory_extract' && task.state === 'completed'
        && !savedMemory.ingested.some(([id]) => id === task.taskId)
      const currentAnalysis = ['memory_extract', 'consistency_check'].includes(task.taskType)
        && currentRevisions.has(task.baseRevisionId) && !activeTask(task.state)
      if (unprocessedExtraction || (currentAnalysis && !task.frozenContext.stagedDraftId)) await settleAnalysis(task)
    }
    await readChapter(chapters.value[0]?.chapterId ?? '')
    background(indexManuscript())
    kickBatch()
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
      memory.value = emptyMemory()
      arcs.value = []
      batch.value = null
      editImpact.value = undefined
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

  async function saveText(editLevel?: EditLevel, canonConfirmed = false, impactTerms = '', canonChange?: Canon) {
    clearTimeout(bufferTimer)
    const entry = chapter.value
    if (!entry || !dirty.value) return
    const before = acceptedText.value
    const input = {
      chapterId: entry.chapterId, baseRevisionId: entry.currentRevisionId,
      source: 'manual_edit' as const, content: text.value, editLevel, canonConfirmed,
    }
    const revision = canonChange ? await desktop.saveCanonRevision(input, canonChange) : await desktop.saveRevision(input)
    acceptedText.value = revision.content
    entry.currentRevisionId = revision.revisionId
    entry.status = 'accepted'
    revisions.value.unshift(revision)
    if (revision.editLevel !== 'surface' || entry.memorySyncStatus === 'processing') entry.memorySyncStatus = 'pending'
    editImpact.value = undefined
    background(indexManuscript())
    if (before && revision.editLevel !== 'surface') background(findImpacts(entry, revision, before, impactTerms))
    flash('已保存正式修订')
    if (canonChange) background(refreshMemory())
  }

  function saveCurrent(level?: EditLevel, confirmed = false, terms = '', canonChange?: Canon) {
    return run(() => saveText(level, confirmed, terms, canonChange))
  }

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
    if (['beat_write','ab_write','batch_write'].includes(task.taskType) && (event.eventType === 'token' || event.eventType === 'completed')) {
      const issue = technicalIssue(task.partialOutput, event.eventType === 'completed', event.payload.finishReason)
      if (issue && !writingIssues.value[task.taskId]) {
        writingIssues.value[task.taskId] = issue
        void taskService.cancelTask(task.taskId, issue).catch((problem) => { error.value = errorMessage(problem) })
      }
    }
    if (!activeTask(task.state) && (task.state !== 'completed' || !writingIssues.value[task.taskId])) {
      subscriptions.get(task.taskId)?.()
      subscriptions.delete(task.taskId)
      void refreshTraces().catch(() => {})
      if (task.frozenContext.batchId) kickBatch()
      else if (task.taskType === 'memory_extract' || task.taskType === 'consistency_check') background(settleAnalysis(task))
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
    const frozenContext = freezeContext(book, entry, revision, profile, instruction, carryDraft, await writerSources(entry, instruction))
    const task = await makeTask(entry, 'beat_write', frozenContext)
    taskId.value = task.taskId
    await taskService.startTask(task.taskId)
  }

  async function makeTask(entry: Chapter, type: TaskInput['taskType'], frozenContext: FrozenContext, key: string = crypto.randomUUID()) {
    const task = await taskService.createTask({ idempotencyKey: key, chapterId: entry.chapterId,
      baseRevisionId: entry.currentRevisionId, taskType: type, frozenContext })
    tasks.value.unshift(task)
    watchTask(task)
    return task
  }

  function setStyle(base: string | null, cartridge: string | null) {
    return run(async () => { styles.value = await desktop.chooseStyle(base, cartridge) })
  }

  function togglePack(pack: Pack) {
    return run(async () => { styles.value = await desktop.enablePack(pack.itemId, !pack.enabled) })
  }

  function importPacks(packs: Pack[]) {
    return run(async () => { styles.value = await desktop.writePacks(packs); flash('配方已写入工程') })
  }

  function saveScene(input: SceneInput) {
    return run(async () => {
      const entry = chapter.value
      if (!entry) throw new Error('请先创建或选择一个章节。')
      if (dirty.value && input.sceneId !== entry.activeSceneId) throw new Error('请先保存当前场景的正文。')
      const saved = await desktop.saveScene(input)
      await desktop.linkScene(entry.chapterId, saved.sceneId)
      scenes.value = await desktop.scenes()
      await readChapters()
      flash('场景已保存')
    })
  }

  function selectScene(sceneId: string | null) {
    return run(async () => {
      if (!chapter.value) throw new Error('请先创建或选择一个章节。')
      if (dirty.value && sceneId !== chapter.value.activeSceneId) throw new Error('请先保存当前场景的正文。')
      await desktop.linkScene(chapter.value.chapterId, sceneId)
      await readChapters()
    })
  }

  function savePrompt(instruction: string) {
    return run(async () => {
      const revision = await desktop.savePrompt(instruction)
      promptHistory.value.unshift(revision)
      flash('已创建提示词新版本')
    })
  }

  async function refreshTraces() { traces.value = await desktop.traces() }

  function runSkillExtraction(excerpt: string) {
    return run(async () => {
      const entry = chapter.value
      const profile = selectedProfile.value
      const book = project.value
      if (!entry || !book || !profile || (profile.needsKey && !profile.hasKey)) throw new Error('请先选择章节和可用模型。')
      const count = Array.from(excerpt.trim()).length
      if (count < 1500 || count > 3000) throw new Error('请粘贴 1500 至 3000 字的样章。')
      const task = await makeTask(entry, 'skill_extract', skillExtraction(book, entry, profile, excerpt.trim()))
      await taskService.startTask(task.taskId)
    })
  }

  function compareModels(firstId: string, secondId: string, instruction: string) {
    return run(async () => {
      const book = project.value
      const entry = chapter.value
      const a = profiles.value.find(profile => profile.id === firstId)
      const b = profiles.value.find(profile => profile.id === secondId)
      if (!book || !entry || !a || !b || a.id === b.id || [a,b].some(p => p.needsKey && !p.hasKey)) {
        throw new Error('请选择一个章节和两份可用的不同模型档案。')
      }
      await saveText()
      const source = revisions.value.find(revision => revision.revisionId === entry.currentRevisionId)
      const ids: ['A' | 'B', 'A' | 'B'] = (crypto.getRandomValues(new Uint8Array(1))[0] ?? 0) % 2 ? ['A', 'B'] : ['B', 'A']
      const id = crypto.randomUUID()
      const shared = freezeContext(book, entry, source, a, instruction, '', await writerSources(entry, instruction))
      const first = await makeTask(entry, 'ab_write', { ...shared, comparison: { id, label: ids[0] } }, id + ':' + ids[0])
      const second = await makeTask(entry, 'ab_write', { ...retarget(shared, b), comparison: { id, label: ids[1] } }, id + ':' + ids[1])
      await Promise.all([taskService.startTask(first.taskId), taskService.startTask(second.taskId)])
    })
  }

  function generate() { return run(() => createAndRun()) }

  function requestInspiration() {
    return run(async () => {
      const book = project.value
      const entry = chapter.value
      const profile = selectedProfile.value
      if (!book || !entry || !profile || (profile.needsKey && !profile.hasKey)) throw new Error('请先配置一个可用的主笔模型。')
      if (inspiration.value && activeTask(inspiration.value.state)) return
      const revision = revisions.value.find(item => item.revisionId === entry.currentRevisionId)
      const sources = await writerSources(entry, draftHint.value)
      const frozen = freezeContext(book, entry, revision, profile, draftHint.value, '', { ...sources, mode: 'inspiration' })
      const task = await makeTask(entry, 'inspiration', frozen)
      await taskService.startTask(task.taskId)
    })
  }

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
      background(indexManuscript())
      if (chapter.value && bookHasReachedTarget(revision)) await analyzeChapter(chapter.value, revision, true)
    })
  }

  function background(work: Promise<unknown>) {
    void work.catch(problem => { error.value = errorMessage(problem) })
  }

  function bookHasReachedTarget(revision: Revision) {
    return !!project.value && revision.cjkCharCount >= project.value.targetChapterChars
  }

  async function refreshMemory() {
    memory.value = await desktop.memory()
    return memory.value
  }

  async function writerSources(entry: Chapter, instruction: string) {
    const snapshot = await refreshMemory()
    const activeScene = scenes.value.find(item => item.sceneId === entry.activeSceneId)
    const scoped = writerMemory(snapshot, entry, activeScene?.details.povCharacterId)
    const priorChapter = chapters.value.find(item => item.chapterOrder === entry.chapterOrder - 1)
    const sharedScene = !!activeScene && priorChapter?.sceneIds?.includes(activeScene.sceneId)
    if (priorChapter?.currentRevisionId && !entry.currentRevisionId && (sharedScene || activeScene?.details.requiresPreviousMemory)
      && priorChapter.memorySyncStatus !== 'synced') {
      throw new Error('上一章的记忆尚未同步。请在上一章完成审查并处理待确认事件。')
    }
    async function readSceneText(from: Chapter) {
      const history = await desktop.revisions(from.chapterId)
      const revision = history.find(item => item.revisionId === from.currentRevisionId)
      if (!revision) return
      const ranges = sceneRanges(history, revision)
      const scoped = ranges.filter(range => range.sceneId === (activeScene?.sceneId ?? null))
      const legacy = activeScene && from.sceneIds?.length === 1 && from.sceneIds.includes(activeScene.sceneId)
        && ranges.every(range => range.sceneId === null)
      const content = legacy ? revision.content : scoped.map(range => revision.content.slice(range.start, range.end)).join('\n\n')
      return { ...revision, content }
    }
    const previous = !scoped.guarded && !entry.currentRevisionId && priorChapter?.currentRevisionId
      && (sharedScene || (!activeScene && !priorChapter.activeSceneId)) ? await readSceneText(priorChapter) : undefined
    const currentSceneText = !scoped.guarded && entry.currentRevisionId ? (await readSceneText(entry))?.content : undefined
    const context = [instruction, activeScene?.title, activeScene?.details.cast, activeScene?.details.location,
      activeScene?.details.conflict, previous?.content.slice(-800)].filter(Boolean).join('\n')
    const terms = [...new Set(scoped.searchableNames)].filter(name => context.includes(name) && cjkQuery(name)).slice(0, 5)
    if (terms.length) await indexManuscript()
    const hits = await Promise.all(terms.map(name => desktop.searchText(cjkQuery(name), name)))
    const arc = arcs.value.find(arc => arc.arcId === entry.arcId)
    return { scene: activeScene, catalog: styles.value, promptRevision: promptHistory.value[0], memory: snapshot,
      previous: previous?.content ? previous : undefined, currentSceneText,
      memoryMatches: hits.flatMap(rows => rows.map(row => row.chapterId)),
      direction: arcDirection(arc, entry), arcTitle: arc?.writerVisible ? arc.title : undefined }
  }

  async function findImpacts(entry: Chapter, revision: Revision, before: string, extra: string) {
    const edited = changeWindow(before, revision.content)
    if (!edited) return
    const relevant = memory.value.canon.flatMap(item => [item.name, ...item.aliases])
      .filter(term => edited.includes(term) && cjkQuery(term))
    const terms = [...new Set([...extra.split(/[\s、，,]+/), ...relevant].filter(term => cjkQuery(term)))].slice(0, 8)
    await indexManuscript()
    const all = (await Promise.all(terms.map(term => desktop.searchText(cjkQuery(term), term, entry.chapterOrder)))).flat()
    const hits = Array.from(new Map(all.map(hit => [hit.chapterId, hit])).values()).sort((a, b) => a.chapterOrder - b.chapterOrder)
    if (chapter.value?.chapterId === entry.chapterId && chapter.value.currentRevisionId === revision.revisionId) {
      editImpact.value = { revisionId: revision.revisionId, hits, terms }
    }
  }

  function requestPolish(start: number, end: number, instruction: string) {
    return run(async () => {
      const entry = chapter.value
      const revision = revisions.value.find(item => item.revisionId === entry?.currentRevisionId)
      const profile = selectedProfile.value
      const book = project.value
      if (!entry || !revision || !book || !profile || (profile.needsKey && !profile.hasKey)) {
        throw new Error('请保存正文并配置可用的编辑模型。')
      }
      if (dirty.value) throw new Error('请保存当前修改，再选中要精修的文字。')
      if (polishTasks.value.some(task => activeTask(task.state))) throw new Error('本章已有待处理的精修任务。')
      const span = makeSpan(revision, start, end)
      const task = await makeTask(entry, 'polish_span', polishContext(book, entry, revision, profile, span, instruction))
      await taskService.startTask(task.taskId)
    })
  }

  function applyPolish(task: GenerationTask, confirmCanon = false) {
    return run(async () => {
      const entry = chapter.value
      if (!entry || dirty.value || task.chapterId !== entry.chapterId) throw new Error('正文有未保存的修改，请保存后重新发起精修。')
      parseReplacement(task.partialOutput)
      const previous = acceptedText.value
      const revision = await desktop.applyPolish(task.taskId, confirmCanon)
      await readChapters()
      await readChapter(task.chapterId)
      background(indexManuscript())
      if (revision.editLevel !== 'surface') background(findImpacts(entry, revision, previous, ''))
      flash('已将精修建议保存为新修订')
    })
  }

  function requestCritic(start?: number, end?: number) {
    return run(async () => {
      const entry = chapter.value
      const book = project.value
      const revision = revisions.value.find(item => item.revisionId === entry?.currentRevisionId)
      const profile = profiles.value.find(item => item.id === memory.value.roles.auditProfileId) ?? selectedProfile.value
      if (!entry || !book || !revision || !profile || (profile.needsKey && !profile.hasKey)) throw new Error('请保存正文并配置可用模型。')
      if (dirty.value) throw new Error('请先保存本章修订。')
      if (criticTasks.value.some(task => activeTask(task.state))) return
      const span = start !== undefined && end !== undefined ? makeSpan(revision, start, end) : undefined
      const task = await makeTask(entry, 'critic', criticContext(book, entry, revision, profile, span))
      await taskService.startTask(task.taskId)
    })
  }

  function saveArc(arc: StageArc) {
    return run(async () => { await desktop.writeArc(arc); arcs.value = await desktop.arcs(); flash('已保存阶段') })
  }
  function assignArc(chapterId: string, arcId: string | null) {
    return run(async () => { await desktop.assignArc(chapterId, arcId); await readChapters() })
  }
  function applyPivot(pivot: Pivot) {
    return run(async () => { await desktop.applyPivot(pivot); arcs.value = await desktop.arcs(); flash('已确认改纲') })
  }

  function exportBook(format: 'story' | 'docx' | 'epub') {
    return run(async () => {
      if (dirty.value) throw new Error('请先保存当前正文。')
      const path = await desktop.exportProject(format)
      if (path) flash('已导出 ' + path.split(/[\\/]/).at(-1))
    })
  }

  function startBatch(input: BatchInput) {
    return run(async () => {
      if (dirty.value) throw new Error('请先保存当前正文。')
      batch.value = await desktop.startBatch(input)
      kickBatch()
    })
  }
  function controlBatch(action: BatchAction, budget?: number) {
    return run(async () => {
      if (!batch.value) return
      if (action === 'stop' && batch.value.step) {
        const task = await desktop.task(batch.value.step.taskId)
        if (task.state === 'queued') await taskService.cancelTask(task.taskId)
        else throw new Error('请等当前模型请求结算，再结束批次。')
        await processCompletedBatchStep(batch.value, await desktop.task(task.taskId))
      }
      batch.value = await desktop.controlBatch(batch.value.batchId, action, budget)
      kickBatch()
    })
  }

  async function processCompletedBatchStep(current: Batch, task: GenerationTask) {
    let problem = writingIssues.value[task.taskId] ?? ''
    let passed = task.state === 'completed' && !problem
    if (passed && task.taskType === 'batch_write') {
      const completed = (await desktop.events(task.taskId, Math.max(0, task.lastEventSeq - 2))).find(event => event.eventType === 'completed')
      problem = technicalIssue(task.partialOutput, true, completed?.payload.finishReason) ?? ''
      if (chapterId.value === task.chapterId && dirty.value) problem = '作者正在编辑本章。请保存并查看后台草稿，再决定是否继续。'
      passed = !problem
    }
    if (passed && (task.taskType === 'consistency_check' || task.taskType === 'memory_extract')) {
      if (!task.frozenContext.stagedDraftId) await settleAnalysis(task)
      if (task.taskType === 'consistency_check') {
        try {
          const staged = task.frozenContext.stagedDraftId ? await desktop.previewBatchDraft(current.batchId) : null
          const source = staged ?? (await desktop.revisions(task.chapterId)).find(item => item.revisionId === task.baseRevisionId)
          if (!source || (task.frozenContext.stagedDraftId && staged?.taskId !== task.frozenContext.stagedDraftId))
            throw new Error('审查稿已变化')
          const findings = auditFindings(task.partialOutput, source.content)
          if (findings.some(finding => finding.severity === 'severe') || task.frozenContext.viewpointAlerts?.length) {
            problem = '出现重大一致性告警。请查看告警，处理后再继续。'
          }
        } catch { problem = '一致性审查报告无法解析'; passed = false }
      } else {
        problem = memory.value.ingested.find(([id]) => id === task.taskId)?.[1] ?? ''
        if (problem) passed = false
      }
    }
    batch.value = await desktop.settleBatchStep(current.batchId, task.taskId, problem, passed)
  }

  function kickBatch() {
    if (!batch.value || !project.value) return
    pumpRequested = true
    if (pumping) return
    pumping = true
    background((async () => {
      while (pumpRequested) {
        pumpRequested = false
        try { await runBatchStep() }
        catch (failure) {
          const current = batch.value
          if (current?.state === 'running') batch.value = await desktop.controlBatch(current.batchId, 'pause', undefined, errorMessage(failure))
          else throw failure
        }
      }
    })().finally(() => { pumping = false; if (pumpRequested) kickBatch() }))
  }

  async function runBatchStep() {
    const book = project.value
    if (!book) return
    const current = await desktop.latestBatch()
    if (project.value?.projectId !== book.projectId) return
    batch.value = current
    if (!current || !['running','paused'].includes(current.state)) return
    if (current.step) {
      const task = await desktop.task(current.step.taskId)
      if (task.state === 'queued' && current.state === 'running') {
        watchTask(task)
        try { await taskService.startTask(task.taskId) }
        catch (failure) {
          await taskService.cancelTask(task.taskId, errorMessage(failure))
          await processCompletedBatchStep(current, await desktop.task(task.taskId))
        }
      } else if (!activeTask(task.state)) {
        await processCompletedBatchStep(current, task)
        pumpRequested = batch.value?.state === 'running'
      } else if (task.state !== 'queued') watchTask(task)
      return
    }
    if (current.state !== 'running') return
    const entry = (await desktop.chapters()).find(item => item.chapterId === current.chapterIds[current.cursor])
    if (!entry) throw new Error('待连写章节已经归档或移除。')
    if (chapterId.value === entry.chapterId && dirty.value) throw new Error('请先保存本章正在编辑的文字。')
    const history = entry.currentRevisionId ? await desktop.revisions(entry.chapterId) : []
    const currentScene = scenes.value.find(scene => scene.sceneId === entry.activeSceneId)
    const source = history.find(item => item.revisionId === entry.currentRevisionId)
    if (current.phase === 'adopt' && current.draftTaskId) {
      const revision = await desktop.adoptBatchDraft(current.batchId, current.draftTaskId)
      await readChapters()
      if (chapterId.value === entry.chapterId && !dirty.value) await readChapter(entry.chapterId)
      await desktop.indexChapter(entry.chapterId, revision.revisionId, cjkIndex(revision.content))
      pumpRequested = true
      return
    }
    if (current.phase === 'advance' || (current.phase === 'extract' && current.auditedRevisionId !== entry.currentRevisionId)
      || (current.phase === 'write' && !current.draftTaskId && source && bookHasReachedTarget(source))) {
      batch.value = await desktop.advanceBatch(current.batchId)
      pumpRequested = batch.value.state === 'running'
      return
    }
    const profileId = current.phase === 'write' ? current.writerProfile : current.phase === 'audit' ? current.auditProfile : current.extractionProfile
    const profile = (await desktop.profiles()).find(item => item.id === profileId && item.rates && (!item.needsKey || item.hasKey))
    if (!profile) throw new Error('后台批次的模型档案或价格已变化，请检查模型设置。')
    const kind = current.phase === 'write' ? 'batch_write' : current.phase === 'audit' ? 'consistency_check' : 'memory_extract'
    const staged = current.draftTaskId ? await desktop.previewBatchDraft(current.batchId) : null
    const auditSource = kind === 'consistency_check' && staged
      ? { revisionId: staged.taskId, parentRevisionId: entry.currentRevisionId, sceneId: entry.activeSceneId, saveKind: 'ai_draft' as const,
        sourceTag: 'batch_write', content: staged.content, cjkCharCount: countHan(staged.content), createdAt: '' } : source
    if (kind !== 'batch_write' && !auditSource) throw new Error('后台检查需要正文。')
    const frozen = kind === 'batch_write'
      ? freezeContext(book, entry, source, profile, current.instruction, staged?.appendix ?? '',
        { ...await writerSources(entry, current.instruction), includeGuardedCarry: true })
      : memoryAnalysis(kind, book, entry, auditSource!, profile, await refreshMemory(),
        currentScene?.details.povCharacterId,
        undefined, viewpointRanges(history, auditSource!))
    const task = await makeTask(entry,kind,{ ...frozen, baseRevisionId: entry.currentRevisionId,
      previousRevisionIds: kind === 'consistency_check' && staged ? (source ? [source.revisionId] : []) : frozen.previousRevisionIds,
      batchId: current.batchId, stagedDraftId: staged?.taskId,
      sceneBindingVersion: kind === 'consistency_check' ? currentScene?.version : frozen.sceneBindingVersion })
    try {
      batch.value = await desktop.reserveBatchStep(current.batchId,task.taskId,profile.id)
    } catch (failure) {
      await taskService.cancelTask(task.taskId)
      throw failure
    }
    if (!batch.value.step) {
      await taskService.cancelTask(task.taskId)
      return
    }
    pumpRequested = true
  }

  async function settleAnalysis(task: GenerationTask) {
    if (!task.baseRevisionId) return
    if (task.state !== 'completed' && !activeTask(task.state) && !writingIssues.value[task.taskId]) {
      const last = (await desktop.events(task.taskId, Math.max(0, task.lastEventSeq - 2))).at(-1)
      if (last?.payload.message) writingIssues.value[task.taskId] = last.payload.message
    }
    if (task.taskType === 'memory_extract' && task.state === 'completed' && !memory.value.ingested.some(([id]) => id === task.taskId)) {
      const revision = (await desktop.revisions(task.chapterId)).find(item => item.revisionId === task.baseRevisionId)
      if (!revision) return
      let result
      try { result = extractCandidates(task.partialOutput, revision.content, task.frozenContext.sceneReview?.ranges) }
      catch { result = { candidates: [], problem: '模型返回的候选无法解析。可重新提取。' } }
      const before = new Set(memory.value.events.map(event => event.eventId))
      await desktop.ingestMemory(task.taskId, result.candidates, result.problem)
      await refreshMemory()
      const added = memory.value.events.filter(event => event.sourceRevisionId === revision.revisionId && event.current
        && event.approval === 'auto_accepted' && !before.has(event.eventId))
      if (added.length) {
        flash(`自动记录了 ${added.length} 条常规记忆`)
        autoFacts.value = added.map(event => event.eventId)
      }
    }
    if (task.state === 'completed' && task.taskType === 'consistency_check') {
      const revision = (await desktop.revisions(task.chapterId)).find(item => item.revisionId === task.baseRevisionId)
      if (revision) {
        try { auditFindings(task.partialOutput, revision.content, task.frozenContext.sceneReview?.ranges) }
        catch { writingIssues.value[task.taskId] = '审查报告格式有误。可重新审查。' }
      }
    }
    await desktop.refreshMemoryStatus(task.chapterId, task.baseRevisionId)
    const updated = (await desktop.chapters()).find(item => item.chapterId === task.chapterId)
    const local = chapters.value.find(item => item.chapterId === task.chapterId)
    if (local?.currentRevisionId === updated?.currentRevisionId && local) local.memorySyncStatus = updated?.memorySyncStatus
  }

  async function analyzeChapter(entry: Chapter, revision: Revision, automatic: boolean, scope?: SceneReview) {
    const book = project.value
    if (!book) return
    const current = await refreshMemory()
    const roles = [
      ['consistency_check', current.roles.auditProfileId], ['memory_extract', current.roles.extractionProfileId],
    ] as const
    const usable = (profile: ModelProfile) => !profile.needsKey || profile.hasKey
    const plans = roles.map(([kind, id]) => {
      const configured = profiles.value.find(profile => profile.id === id && usable(profile))
      const fallback = !automatic && !id ? selectedProfile.value : undefined
      return { kind, profile: configured ?? (fallback && usable(fallback) ? fallback : undefined) }
    }).filter(({ profile }) => profile !== undefined)
    if (!plans.length) {
      if (!automatic) throw new Error('请在记忆库配置审查与提取模型，或选择一个可用模型。')
      return
    }
    if (automatic && plans.length < 2) return
    const previous = tasks.value.filter(task => task.baseRevisionId === revision.revisionId && !task.frozenContext.stagedDraftId
      && task.frozenContext.sceneReview?.sceneId === scope?.sceneId)
    const pending = plans.filter(({ kind }) => !previous.some(task => task.taskType === kind &&
      (activeTask(task.state) || automatic || (task.state === 'completed' &&
        !writingIssues.value[task.taskId] && !current.ingested.some(([id, problem]) => id === task.taskId && problem)))))
    if (!pending.length) return
    const pov = scenes.value.find(item => item.sceneId === (scope?.sceneId ?? entry.activeSceneId))?.details.povCharacterId
    const viewpoints = scope ? scope.ranges.map(range => ({ ...range, povCharacterId: pov ?? null }))
      : viewpointRanges(await desktop.revisions(entry.chapterId), revision)
    const analysisTasks = await Promise.all(pending.map(({ kind, profile }) =>
      makeTask(entry, kind, memoryAnalysis(kind, book, entry, revision, profile!, current, pov, scope, viewpoints))))
    const started = await Promise.allSettled(analysisTasks.map(task => taskService.startTask(task.taskId)))
    for (const [index, result] of started.entries()) {
      if (result.status === 'fulfilled') continue
      const task = analysisTasks[index]
      if (task) await taskService.cancelTask(task.taskId, errorMessage(result.reason))
    }
    await desktop.refreshMemoryStatus(entry.chapterId, revision.revisionId)
    const latest = (await desktop.chapters()).find(item => item.chapterId === entry.chapterId)
    if (latest?.currentRevisionId === entry.currentRevisionId) entry.memorySyncStatus = latest?.memorySyncStatus
  }

  function viewpointRanges(history: Revision[], revision: Revision) {
    return sceneRanges(history, revision).map(({ start, end, sceneId }) => ({
      start, end, povCharacterId: scenes.value.find(item => item.sceneId === sceneId)?.details.povCharacterId ?? null,
    }))
  }

  function completeChapter() {
    return run(async () => {
      await saveText()
      const entry = chapter.value
      const revision = revisions.value.find(item => item.revisionId === entry?.currentRevisionId)
      if (!entry || !revision?.content.trim()) throw new Error('请先保存本章正文。')
      await analyzeChapter(entry, revision, false)
    })
  }

  function completeScene() {
    return run(async () => {
      const active = scene.value
      if (!active) throw new Error('请选择一个已保存的场景。')
      await saveText()
      let found = false
      for (const entry of chapters.value.filter(item => item.sceneIds?.includes(active.sceneId) && item.currentRevisionId)) {
        const history = await desktop.revisions(entry.chapterId)
        const revision = history.find(item => item.revisionId === entry.currentRevisionId)
        if (!revision) continue
        const ranges = sceneRanges(history, revision).filter(range => range.sceneId === active.sceneId)
          .map(({ start, end }) => ({ start, end }))
        if (!ranges.length && entry.sceneIds?.length === 1 && !history.some(item => item.sceneId) && revision.content.trim()) {
          ranges.push({ start: 0, end: revision.content.length })
        }
        if (!ranges.length) continue
        found = true
        await analyzeChapter(entry, revision, false, { sceneId: active.sceneId, ranges })
      }
      if (!found) throw new Error('此场景还没有正式正文。')
    })
  }

  function undoAutomaticMemory() {
    return run(async () => {
      for (const id of autoFacts.value) await desktop.reviewFact(id, 'undo')
      await refreshMemory()
      await readChapters()
      flash('已撤销自动记忆')
    })
  }

  function indexManuscript() {
    if (indexing) return indexing
    const scope = project.value?.projectId
    indexing = (async () => {
      while (scope && scope === project.value?.projectId) {
        const batch = await desktop.staleIndexes()
        if (!batch.length) return
        for (const entry of batch) {
          if (scope !== project.value?.projectId) return
          await desktop.indexChapter(entry.chapterId, entry.revisionId, cjkIndex(entry.content))
        }
      }
    })().finally(() => { indexing = undefined })
    return indexing
  }

  async function searchManuscript(text: string) {
    const exact = text.trim()
    const phrase = cjkQuery(exact)
    if (!phrase) return []
    await indexManuscript()
    return desktop.searchText(phrase, exact)
  }

  async function canonImpact(entry: Canon): Promise<CanonImpact> {
    const previous = memory.value.canon.find(item => item.entryId === entry.entryId)
    const names = [...new Set([entry.name, ...entry.aliases, ...(previous ? [previous.name, ...previous.aliases] : [])])]
    await indexManuscript()
    const hits = (await Promise.all(names.filter(cjkQuery).map(term => desktop.searchText(cjkQuery(term), term)))).flat()
    const facts = memory.value.events.filter(fact => fact.current && fact.approval !== 'rejected'
      && (fact.subjects.includes(entry.entryId) || names.some(name => name && fact.summary.includes(name))))
    const related = new Set([entry.entryId, ...facts.map(fact => fact.eventId)])
    const notes = memory.value.notes.filter(note => note.status !== 'deprecated'
      && (note.relatedIds.some(id => related.has(id)) || names.some(name => name && note.body.includes(name))))
    return { hits: [...new Map(hits.map(hit => [hit.chapterId, hit])).values()].sort((a, b) => a.chapterOrder - b.chapterOrder), facts, notes }
  }
  function saveCanon(entry: Canon) {
    return run(async () => {
      if (entry.entryId) {
        const frozen = JSON.parse(JSON.stringify(entry)) as Canon
        canonProposal.value = { entry: frozen, impact: await canonImpact(frozen) }
        return
      }
      await desktop.writeCanon(entry)
      await refreshMemory()
      flash('设定已存入工程')
    })
  }
  function confirmCanonChange() {
    return run(async () => {
      if (!canonProposal.value) return
      await desktop.writeCanon(canonProposal.value.entry)
      await refreshMemory()
      canonProposal.value = null
      flash('权威设定已更新')
    })
  }
  function saveNote(note: Note) { return run(async () => { await desktop.writeNote(note); await refreshMemory(); flash('备注已保存') }) }
  function saveKnowledge(entry: Knowledge) { return run(async () => { await desktop.writeKnowledge(entry); await refreshMemory(); flash('已保存人物知情状态') }) }
  function saveRoles(roles: MemoryRoles) { return run(async () => { await desktop.writeMemoryRoles(roles); await refreshMemory(); flash('已保存模型分工') }) }
  function reviewFact(fact: Fact, action: 'accept' | 'reject' | 'undo', audience?: Audience) {
    return run(async () => { await desktop.reviewFact(fact.eventId, action, audience); await refreshMemory(); await readChapters() })
  }

  return {
    project, chapters, archived, chapter, chapterId, profiles, profileId, selectedProfile, revisions, history, text, acceptedText,
    draftHint, error, notice, busy, dirty, currentCount, tasks, chapterTasks, taskId, currentTask, currentActive,
    queuedCount, writingIssues, initialize, openWorkspace, closeWorkspace, renameWorkspace, readProfiles, openChapter, addChapter,
    renameChapter, moveChapter, archiveSelected, restoreChapter, saveCurrent, restoreRevision, generate, startQueued,
    cancel, resumeTask, acceptDraft, flash, styles, scenes, scene, sceneIntent, sceneStyles, promptHistory, traces,
    setStyle, togglePack, importPacks, saveScene, selectScene, savePrompt, refreshTraces, runSkillExtraction, compareModels,
    memory, memoryPending, chapterFindings, chapterAudits, chapterAnalysis, saveCanon, saveNote, saveKnowledge, saveRoles, reviewFact, completeChapter, completeScene, autoFacts, undoAutomaticMemory,
    searchManuscript,
    arcs, saveArc, assignArc, applyPivot, editImpact, polishTasks, requestPolish, applyPolish, criticTasks, criticFindings, requestCritic,
    batch, startBatch, controlBatch, exportBook, inspiration, requestInspiration, canonImpact, canonProposal, confirmCanonChange,
  }
})

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
import { builtins } from '../core/skills/builtins'
import { skillExtraction } from '../core/skills/extraction'
import { cjkIndex, cjkQuery } from '../core/db/fts'
import { auditFindings, emptyMemory, extractCandidates, memoryAnalysis, writerMemory,
  type Audience, type Canon, type Fact, type Knowledge, type MemoryRoles, type Note, type SearchHit } from '../core/narrative/memory'
import { arcDirection, changeWindow, criticContext, makeSpan, parseReplacement, polishContext,
  type EditLevel, type Pivot, type StageArc } from '../core/narrative/editing'

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
  const styles = ref<StyleCatalog>({ packs: [], activeBaseSkillId: null, activePackId: null })
  const scenes = ref<Scene[]>([])
  const promptHistory = ref<PromptRevision[]>([])
  const traces = ref<Trace[]>([])
  const memory = ref(emptyMemory())
  const arcs = ref<StageArc[]>([])
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
  const currentCount = computed(() => countHan(text.value))
  const dirty = computed(() => text.value !== acceptedText.value)
  const history = computed(() => revisions.value.filter((revision) => revision.saveKind !== 'auto_buffer'))
  const queuedCount = computed(() => tasks.value.filter((task) => activeTask(task.state)).length)
  const scene = computed(() => scenes.value.find((item) => item.sceneId === chapter.value?.activeSceneId))
  const sceneIntent = computed(() => deriveIntent(scene.value, draftHint.value, acceptedText.value))
  const sceneStyles = computed(() => activeStyles(styles.value, scene.value, sceneIntent.value, draftHint.value))
  const memoryPending = computed(() => memory.value.events.filter(event => event.approval === 'pending' && event.current).length)
  const chapterAnalysis = computed(() => tasks.value.filter(task => task.chapterId === chapterId.value
    && task.baseRevisionId === chapter.value?.currentRevisionId && ['memory_extract', 'consistency_check'].includes(task.taskType)))
  const chapterAudits = computed(() => chapterAnalysis.value.filter(task => task.taskType === 'consistency_check'))
  const criticTasks = computed(() => tasks.value.filter(task => task.chapterId === chapterId.value &&
    task.baseRevisionId === chapter.value?.currentRevisionId && task.taskType === 'critic'))
  const polishTasks = computed(() => tasks.value.filter(task => task.chapterId === chapterId.value && task.taskType === 'polish_span'))
  const chapterFindings = computed(() => {
    const task = chapterAudits.value.find(entry => entry.state === 'completed')
    try {
      return task ? auditFindings(task.partialOutput, acceptedText.value) : []
    } catch { return [] }
  })
  const criticFindings = computed(() => {
    const last = criticTasks.value.find(task => task.state === 'completed')
    try { return last ? auditFindings(last.partialOutput, acceptedText.value) : [] }
    catch { return [] }
  })

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
    const [catalog, savedScenes, prompts, storedTasks, savedMemory, savedArcs] = await Promise.all([
      desktop.writePacks(builtins, true), desktop.scenes(), desktop.prompts(), desktop.tasks(), desktop.memory(), desktop.arcs(), readChapters(),
    ])
    styles.value = catalog
    scenes.value = savedScenes
    promptHistory.value = prompts
    tasks.value = storedTasks
    writingIssues.value = {}
    memory.value = savedMemory
    arcs.value = savedArcs
    editImpact.value = undefined
    for (const task of tasks.value.filter((item) => activeTask(item.state))) watchTask(task)
    const currentRevisions = new Set(chapters.value.map(item => item.currentRevisionId))
    for (const task of storedTasks) {
      const unprocessedExtraction = task.taskType === 'memory_extract' && task.state === 'completed'
        && !savedMemory.ingested.some(([id]) => id === task.taskId)
      const currentAnalysis = ['memory_extract', 'consistency_check'].includes(task.taskType)
        && currentRevisions.has(task.baseRevisionId) && !activeTask(task.state)
      if (unprocessedExtraction || currentAnalysis) await settleAnalysis(task)
    }
    await readChapter(chapters.value[0]?.chapterId ?? '')
    background(indexManuscript())
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

  async function saveText(editLevel?: EditLevel, canonConfirmed = false, impactTerms = '') {
    clearTimeout(bufferTimer)
    const entry = chapter.value
    if (!entry || !dirty.value) return
    const before = acceptedText.value
    const revision = await desktop.saveRevision({
      chapterId: entry.chapterId, baseRevisionId: entry.currentRevisionId,
      source: 'manual_edit', content: text.value, editLevel, canonConfirmed,
    })
    acceptedText.value = revision.content
    entry.currentRevisionId = revision.revisionId
    entry.status = 'accepted'
    revisions.value.unshift(revision)
    if (revision.editLevel !== 'surface' || entry.memorySyncStatus === 'processing') entry.memorySyncStatus = 'pending'
    editImpact.value = undefined
    background(indexManuscript())
    if (before && revision.editLevel !== 'surface') background(findImpacts(entry, revision, before, impactTerms))
    flash('已保存正式修订')
  }

  function saveCurrent(level?: EditLevel, confirmed = false, terms = '') { return run(() => saveText(level, confirmed, terms)) }

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
    if ((task.taskType === 'beat_write' || task.taskType === 'ab_write') && (event.eventType === 'token' || event.eventType === 'completed')) {
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
      if (task.taskType === 'memory_extract' || task.taskType === 'consistency_check') background(settleAnalysis(task))
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
    const terms = [...new Set(scoped.searchableNames)].filter(name => instruction.includes(name) && cjkQuery(name)).slice(0, 5)
    if (terms.length) await indexManuscript()
    const hits = await Promise.all(terms.map(name => desktop.searchText(cjkQuery(name), name)))
    return { scene: activeScene, catalog: styles.value, promptRevision: promptHistory.value[0], memory: snapshot,
      memoryMatches: hits.flatMap(rows => rows.map(row => row.chapterId)),
      direction: arcDirection(arcs.value.find(arc => arc.arcId === entry.arcId), entry) }
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

  function requestCritic() {
    return run(async () => {
      const entry = chapter.value
      const book = project.value
      const revision = revisions.value.find(item => item.revisionId === entry?.currentRevisionId)
      const profile = profiles.value.find(item => item.id === memory.value.roles.auditProfileId) ?? selectedProfile.value
      if (!entry || !book || !revision || !profile || (profile.needsKey && !profile.hasKey)) throw new Error('请保存正文并配置可用模型。')
      if (dirty.value) throw new Error('请先保存本章修订。')
      if (criticTasks.value.some(task => activeTask(task.state))) return
      const task = await makeTask(entry, 'critic', criticContext(book, entry, revision, profile))
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
      try { result = extractCandidates(task.partialOutput, revision.content) }
      catch { result = { candidates: [], problem: '模型返回的候选无法解析。可重新提取。' } }
      await desktop.ingestMemory(task.taskId, result.candidates, result.problem)
      await refreshMemory()
    }
    if (task.state === 'completed' && task.taskType === 'consistency_check') {
      const revision = (await desktop.revisions(task.chapterId)).find(item => item.revisionId === task.baseRevisionId)
      if (revision) {
        try { auditFindings(task.partialOutput, revision.content) }
        catch { writingIssues.value[task.taskId] = '审查报告格式有误。可重新审查。' }
      }
    }
    await desktop.refreshMemoryStatus(task.chapterId, task.baseRevisionId)
    const updated = (await desktop.chapters()).find(item => item.chapterId === task.chapterId)
    const local = chapters.value.find(item => item.chapterId === task.chapterId)
    if (local?.currentRevisionId === updated?.currentRevisionId && local) local.memorySyncStatus = updated?.memorySyncStatus
  }

  async function analyzeChapter(entry: Chapter, revision: Revision, automatic: boolean) {
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
    const previous = tasks.value.filter(task => task.baseRevisionId === revision.revisionId)
    const pending = plans.filter(({ kind }) => !previous.some(task => task.taskType === kind &&
      (activeTask(task.state) || automatic || (task.state === 'completed' &&
        !writingIssues.value[task.taskId] && !current.ingested.some(([id, problem]) => id === task.taskId && problem)))))
    if (!pending.length) return
    const pov = scenes.value.find(item => item.sceneId === entry.activeSceneId)?.details.povCharacterId
    const analysisTasks = await Promise.all(pending.map(({ kind, profile }) =>
      makeTask(entry, kind, memoryAnalysis(kind, book, entry, revision, profile!, current, pov))))
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

  function completeChapter() {
    return run(async () => {
      await saveText()
      const entry = chapter.value
      const revision = revisions.value.find(item => item.revisionId === entry?.currentRevisionId)
      if (!entry || !revision?.content.trim()) throw new Error('请先保存本章正文。')
      await analyzeChapter(entry, revision, false)
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

  function saveCanon(entry: Canon) { return run(async () => { await desktop.writeCanon(entry); await refreshMemory(); flash('设定已存入工程') }) }
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
    memory, memoryPending, chapterFindings, chapterAudits, chapterAnalysis, saveCanon, saveNote, saveKnowledge, saveRoles, reviewFact, completeChapter,
    searchManuscript,
    arcs, saveArc, assignArc, applyPivot, editImpact, polishTasks, requestPolish, applyPolish, criticTasks, criticFindings, requestCritic,
  }
})

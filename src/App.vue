<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { isTauri } from '@tauri-apps/api/core'
import {
  ArrowDown, ArrowLeft, ArrowRight, ArrowUp, BookOpen, ChevronDown, CircleAlert,
  Beaker, Binoculars, Clock3, Copy, Dices, FilePlus2, FolderOpen, Library, PanelRight, PenLine, Plus, Route, Settings2, Sparkles, Square, X, Zap,
} from 'lucide-vue-next'
import ModelSettings from './components/ModelSettings.vue'
import SceneControls from './components/SceneControls.vue'
import PacksView from './views/PacksView.vue'
import LabView from './views/LabView.vue'
import MemoryView from './views/MemoryView.vue'
import OutlineView from './views/OutlineView.vue'
import PanoramaView from './views/PanoramaView.vue'
import EditingTools from './components/EditingTools.vue'
import CanonImpact from './components/CanonImpact.vue'
import WritingContext from './components/WritingContext.vue'
import { activeTask, type Chapter, type GenerationTask, type TaskState } from './core/contracts/types'
import { sceneRanges } from './core/narrative/scenes'
import type { Canon, CanonImpact as CanonImpactData } from './core/narrative/memory'
import { countHan } from './core/quality/hardGate'
import { suggestedEdit, type EditLevel } from './core/narrative/editing'
import { useStudio } from './stores/studio'

const studio = useStudio()
const settings = ref(false)
const title = ref('')
const creatingBook = ref(false)
const chapterForm = ref(false)
const newChapterTitle = ref('')
const panel = ref<'draft' | 'history' | 'review' | 'editing'>('draft')
const showInspector = ref(true)
const view = ref<'studio' | 'packs' | 'lab' | 'memory' | 'outline' | 'panorama'>('studio')
const manuscript = ref<HTMLTextAreaElement>()
const selection = ref({ start: 0, end: 0 })
const revisionDialog = ref<HTMLDialogElement>()
const canonDialog = ref<HTMLDialogElement>()
const editLevel = ref<EditLevel>('semantic')
const confirmed = ref(false)
const impactTerms = ref('')
const canonTarget = ref('')
const canonBody = ref('')
const revisionCanon = ref<{ entry: Canon; impact: CanonImpactData } | null>(null)
const previewingCanon = ref(false)
const reviewEventId = ref('')
const openBatch = ref(false)
const showInspiration = ref(false)
watch(() => studio.project?.projectId, () => { view.value = 'studio' })
watch(() => [studio.chapterId, studio.chapter?.currentRevisionId], () => { selection.value = { start: 0, end: 0 }; showInspiration.value = false })
watch(() => studio.canonProposal, async proposal => {
  await nextTick()
  if (proposal && !canonDialog.value?.open) canonDialog.value?.showModal()
  else if (!proposal && canonDialog.value?.open) canonDialog.value.close()
})
watch([canonTarget, canonBody, editLevel], () => { revisionCanon.value = null; confirmed.value = false })
const chapterProgress = computed(() => {
  const target = studio.project?.targetChapterChars ?? 2000
  return Math.min(100, Math.round(studio.currentCount / target * 100))
})
const states: Record<TaskState, string> = {
  created: '已创建', queued: '待发出', running: '连接模型', streaming: '正在续写',
  completed: '待采纳', failed: '写作中断', cancelled: '已停止', interrupted: '已中断', outcome_unknown: '远端结果未知',
}
const recovered = computed(() => studio.currentTask && ['outcome_unknown', 'interrupted', 'failed', 'cancelled'].includes(studio.currentTask.state))
const date = new Intl.DateTimeFormat('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' })
const reviewCount = computed(() => studio.chapterFindings.filter(item => item.severity === 'severe').length
  + studio.memory.events.filter(item => item.chapterId === studio.chapterId && item.current && item.approval === 'pending').length)
const sync = { pending: '待同步', processing: '处理中', synced: '已同步', needs_confirm: '待确认' }
const stage = computed(() => studio.arcs.find(arc => arc.arcId === studio.chapter?.arcId))
const activeCartridge = computed(() => studio.styles.packs.find(pack => pack.itemId === studio.styles.activePackId))
const chapterGroups = computed(() => {
  const groups: { key: string; name: string; chapters: Chapter[] }[] = []
  for (const chapter of studio.chapters) {
    const key = chapter.arcId ?? ''
    const last = groups.at(-1)
    if (last?.key === key) last.chapters.push(chapter)
    else groups.push({ key, name: studio.arcs.find(arc => arc.arcId === key)?.title ?? '未分卷', chapters: [chapter] })
  }
  return groups
})
const inspirationIdeas = computed(() => (studio.inspiration?.partialOutput ?? '').split(/\r?\n/)
  .map(line => line.replace(/^\s*(?:[-*•]|\d+[.、)])[ \t]*/, '').trim()).filter(Boolean).slice(0, 3))
const checks = computed(() => {
  if (panel.value !== 'review' || studio.dirty) return []
  const source = studio.acceptedText
  const findings = studio.chapterFindings.filter(finding => finding.severity === 'severe')
    .sort((a, b) => a.anchor.startOffset - b.anchor.startOffset)
  const segments: { content: string; issue?: typeof findings[number] }[] = []
  let offset = 0
  for (const issue of findings) {
    const { startOffset, endOffset, quote } = issue.anchor
    if (startOffset < offset || source.slice(startOffset, endOffset) !== quote) continue
    segments.push({ content: source.slice(offset, startOffset) }, { content: quote, issue })
    offset = endOffset
  }
  if (offset) segments.push({ content: source.slice(offset) })
  return segments
})

function batchPage() { openBatch.value = true; view.value = 'panorama' }
async function openScene(chapter: Chapter, sceneId: string) {
  if (studio.chapterId !== chapter.chapterId) await studio.openChapter(chapter.chapterId)
  if (studio.chapterId !== chapter.chapterId || studio.error) return
  if (studio.chapter?.activeSceneId !== sceneId) await studio.selectScene(sceneId)
  const head = studio.revisions.find(item => item.revisionId === studio.chapter?.currentRevisionId)
  const range = head ? sceneRanges(studio.revisions, head).find(item => item.sceneId === sceneId) : undefined
  if (head && range) await locate(chapter.chapterId, head.content.slice(range.start, Math.min(range.end, range.start + 36)), range.start)
}

async function locate(chapterId: string, quote: string, preferred?: number) {
  if (studio.chapterId !== chapterId) await studio.openChapter(chapterId)
  if (studio.chapterId !== chapterId || studio.error) return
  view.value = 'studio'
  await nextTick()
  if (!manuscript.value && checks.value.length) { panel.value = 'editing'; await nextTick() }
  const editor = manuscript.value
  if (!editor || !quote) return
  const start = preferred !== undefined && editor.value.slice(preferred, preferred + quote.length) === quote
    ? preferred : editor.value.indexOf(quote)
  if (start < 0) { studio.flash('原文位置已变化'); return }
  editor.focus()
  editor.setSelectionRange(start, start + quote.length)
  readSelection()
  const line = Number.parseFloat(getComputedStyle(editor).lineHeight)
  if (Number.isFinite(line)) editor.scrollTop = Math.max(0, (editor.value.slice(0, start).split('\n').length - 3) * line)
}
async function rewriteFinding(quote: string, start: number) {
  panel.value = 'editing'
  showInspector.value = true
  await locate(studio.chapterId, quote, start)
  const current = selection.value
  if (current.start < current.end) await studio.requestPolish(current.start, current.end, '修正这处一致性或视角问题，保留人物语气与原段落作用。')
}
function readSelection() {
  const editor = manuscript.value
  if (editor) selection.value = { start: editor.selectionStart, end: editor.selectionEnd }
}
async function showReview(chapterId: string) {
  if (studio.chapterId !== chapterId) await studio.openChapter(chapterId)
  if (studio.chapterId !== chapterId || studio.error) return
  view.value = 'studio'
  panel.value = 'review'
  showInspector.value = true
}
function chooseRevision() {
  if (!studio.dirty || studio.busy) return
  studio.error = ''
  editLevel.value = suggestedEdit(studio.acceptedText, studio.text)
  confirmed.value = false
  impactTerms.value = ''
  canonTarget.value = ''
  canonBody.value = ''
  revisionDialog.value?.showModal()
}
async function submitRevision() {
  if (editLevel.value === 'canon' && (!revisionCanon.value || !confirmed.value)) return
  await studio.saveCurrent(editLevel.value, confirmed.value, impactTerms.value,
    editLevel.value === 'canon' ? revisionCanon.value?.entry : undefined)
  if (!studio.error) {
    revisionDialog.value?.close()
    if (editLevel.value !== 'surface') { panel.value = 'editing'; showInspector.value = true }
  }
}
function selectCanonTarget() {
  canonBody.value = studio.memory.canon.find(item => item.entryId === canonTarget.value)?.body ?? ''
}
async function previewCanonRevision() {
  const entry = studio.memory.canon.find(item => item.entryId === canonTarget.value)
  if (!entry || !canonBody.value.trim() || entry.body === canonBody.value) return
  previewingCanon.value = true
  const draft = { ...entry, body: canonBody.value }
  try {
    const impact = await studio.canonImpact(draft)
    if (editLevel.value === 'canon' && canonTarget.value === draft.entryId && canonBody.value === draft.body)
      revisionCanon.value = { entry: draft, impact }
  } catch (error) { studio.error = error instanceof Error ? error.message : '读取影响范围失败。' }
  finally { previewingCanon.value = false }
}

function keyboard(event: KeyboardEvent) {
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 's' && studio.project) {
    event.preventDefault()
    chooseRevision()
  }
}
onMounted(() => {
  window.addEventListener('keydown', keyboard)
  if (isTauri()) void studio.initialize()
})
onUnmounted(() => window.removeEventListener('keydown', keyboard))

async function createBook() {
  if (!title.value.trim()) return
  await studio.openWorkspace(title.value)
  if (studio.project) creatingBook.value = false
}
async function createChapter() {
  await studio.addChapter(newChapterTitle.value, studio.chapterId || undefined)
  if (!studio.error) chapterForm.value = false
  newChapterTitle.value = ''
}
async function model() {
  settings.value = true
  if (isTauri()) await studio.readProfiles().catch(() => {})
}
function askInspiration() {
  if (!studio.selectedProfile || (studio.selectedProfile.needsKey && !studio.selectedProfile.hasKey)) { void model(); return }
  showInspiration.value = true
  void studio.requestInspiration()
}
function generate() {
  const selected = studio.selectedProfile
  if (!selected || selected.needsKey && !selected.hasKey) {
    void model()
    return
  }
  panel.value = 'draft'
  showInspector.value = true
  void studio.generate()
}
function renameChapter(event: Event) {
  const field = event.target as HTMLInputElement
  if (field.value !== studio.chapter?.title) void studio.renameChapter(field.value)
}
function renameBook() {
  if (!studio.project) return
  const value = window.prompt('修改书名', studio.project.title)
  if (value?.trim() && value !== studio.project.title) void studio.renameWorkspace(value)
}
function archive() {
  if (studio.chapter && window.confirm(`归档「${studio.chapter.title}」？可从归档列表恢复。`)) void studio.archiveSelected()
}
function recover(task: GenerationTask, action: 'keep_draft' | 'continue_from_draft' | 'rerun_new_task') {
  if (action !== 'keep_draft' && !window.confirm('将创建一次新的模型请求。先前中断的请求可能已计费。继续吗？')) return
  void studio.resumeTask(task, action)
}
async function copyDraft(task: GenerationTask) {
  try {
    await navigator.clipboard.writeText([task.frozenContext.carryDraft, task.partialOutput].filter(Boolean).join('\n\n'))
    studio.flash('草稿已复制')
  } catch {
    studio.error = '无法访问剪贴板。请选中草稿正文并复制。'
  }
}
</script>

<template>
  <div class="app-shell" :class="{ 'on-desk': studio.project }">
    <header class="app-header">
      <button type="button" class="wordmark" :aria-label="studio.project ? '返回工程选择' : 'StoryForge'" @click="studio.project && studio.closeWorkspace()">
        <span class="emblem"><BookOpen :size="19" :stroke-width="1.9" /></span>
        <strong>StoryForge</strong>
      </button>
      <div v-if="studio.project" class="header-project">
        <span class="header-rule"></span>
        <button type="button" class="book-name flat-button" title="修改书名" @click="renameBook">{{ studio.project.title }} <ChevronDown :size="13" /></button>
        <button type="button" class="flat-button stage-header" aria-label="大纲与阶段进度" @click="view = 'outline'">
          <Route :size="15" /><span>{{ stage?.title || '大纲' }}</span>
          <span v-if="stage?.milestones.length" class="stage-count">{{ stage.milestones.filter(item => item.done).length }}/{{ stage.milestones.length }}</span>
        </button>
        <button v-if="activeCartridge" type="button" class="flat-button header-cartridge" @click="view = 'packs'">{{ activeCartridge.name }}</button>
      </div>
      <div class="header-actions">
        <button v-if="studio.project" class="flat-button header-budget" type="button" @click="batchPage">
          <Zap :size="14" /> <span>托管</span><span v-if="studio.batch && ['running','paused'].includes(studio.batch.state)">{{ studio.batch.cursor }}/{{ studio.batch.chapterIds.length }}</span></button>
        <button v-if="studio.project" class="flat-button header-memory" type="button" @click="reviewEventId = ''; view = 'memory'">
          记忆库<span v-if="studio.memoryPending" class="pending-count">{{ studio.memoryPending }}</span></button>
        <button v-if="studio.project" type="button" class="flat-button desktop-only" :disabled="studio.busy" @click="studio.openWorkspace()"><FolderOpen :size="16" /> 打开工程</button>
        <button type="button" class="flat-button" aria-label="模型设置" @click="model"><Settings2 :size="17" /><span>{{ studio.selectedProfile?.name ?? '模型设置' }}</span></button>
        <button v-if="studio.project && view === 'studio'" class="icon-button" type="button" :aria-pressed="showInspector" aria-label="切换右侧面板" @click="showInspector = !showInspector"><PanelRight :size="18" /></button>
      </div>
    </header>
    <nav v-if="studio.project" class="workspace-nav" aria-label="工作台导航">
      <button class="icon-button" :class="{ selected: view === 'studio' || view === 'outline' }" type="button" title="写作台" aria-label="写作台" @click="view = 'studio'"><PenLine :size="18" /></button>
      <button class="icon-button" :class="{ selected: view === 'panorama' || view === 'memory' }" type="button" title="全景库" aria-label="全景库"
        @click="openBatch = false; view = 'panorama'"><Binoculars :size="18" /><i v-if="studio.memoryPending || studio.batch?.state === 'paused'" class="review-dot"></i></button>
      <button class="icon-button" :class="{ selected: view === 'packs' }" type="button" title="卡带与配方" aria-label="卡带与配方" @click="view = 'packs'"><Library :size="18" /></button>
      <button class="icon-button" :class="{ selected: view === 'lab' }" type="button" title="实验室" aria-label="实验室" @click="view = 'lab'"><Beaker :size="18" /></button>
    </nav>

    <main v-if="!studio.project" class="launcher">
      <div class="launch-intro">
        <p class="micro">长篇小说工作台</p>
        <h1>开始写下一页</h1>
        <p class="launch-description">打开已有的 .story 工程，或新建一本小说。</p>
        <div v-if="!creatingBook" class="launch-actions">
          <button class="primary-button" type="button" :disabled="studio.busy" @click="creatingBook = true"><FilePlus2 :size="17" /> 新建小说 <ArrowRight :size="16" /></button>
          <button class="secondary-button" type="button" :disabled="studio.busy" @click="studio.openWorkspace()"><FolderOpen :size="17" /> 打开工程</button>
        </div>
        <form v-else class="book-form" @submit.prevent="createBook">
          <label for="book-title">书名</label>
          <input id="book-title" v-model="title" required maxlength="120" placeholder="给小说起一个名字" autofocus>
          <div class="launch-actions"><button type="submit" class="primary-button" :disabled="studio.busy">选择保存位置 <ArrowRight :size="16" /></button>
            <button type="button" class="flat-button" :disabled="studio.busy" @click="creatingBook = false">返回</button>
          </div>
        </form>
      </div>
      <div class="launch-art" aria-hidden="true">
        <div class="book-art"><div class="book-spine"></div><BookOpen :size="66" :stroke-width=".85" /><i></i><i></i><i></i></div>
      </div>
    </main>

    <PacksView v-else-if="view === 'packs'" />
    <LabView v-else-if="view === 'lab'" @settings="model" @open="locate" />
    <MemoryView v-else-if="view === 'memory'" :review-event-id="reviewEventId" @open="locate" />
    <OutlineView v-else-if="view === 'outline'" />
    <PanoramaView v-else-if="view === 'panorama'" :open-batch="openBatch" @open="locate" @review="showReview" @memory="reviewEventId = ''; view = 'memory'" />
    <main v-else class="studio-layout" :class="{ compact: !showInspector }">
      <aside class="chapter-pane" aria-label="章节目录">
        <div class="aside-head"><strong>章节</strong><button type="button" class="icon-button" aria-label="添加章节" @click="chapterForm = !chapterForm"><Plus :size="18" /></button></div>
        <form v-if="chapterForm" class="chapter-form" @submit.prevent="createChapter">
          <input v-model="newChapterTitle" maxlength="120" placeholder="新章标题" aria-label="新章标题">
          <button class="primary-button" type="submit" :disabled="studio.busy">创建</button>
        </form>
        <nav class="chapter-list">
          <template v-for="group in chapterGroups" :key="group.chapters[0]?.chapterId">
            <div v-if="chapterGroups.length > 1 || group.key" class="volume-label">{{ group.name }}</div>
            <template v-for="chapter in group.chapters" :key="chapter.chapterId">
          <button type="button" :class="{ selected: studio.chapterId === chapter.chapterId }"
            :aria-current="studio.chapterId === chapter.chapterId ? 'page' : undefined" @click="studio.openChapter(chapter.chapterId)">
            <span class="chapter-no">{{ String(chapter.chapterOrder).padStart(2, '0') }}</span>
            <span>{{ chapter.title }}</span><span v-if="chapter.currentRevisionId" class="chapter-dot"></span>
          </button>
              <div v-if="chapter.sceneIds?.length" class="chapter-scenes">
                <button v-for="sceneId in chapter.sceneIds" :key="sceneId" type="button"
                  :class="{ selected: studio.chapterId === chapter.chapterId && chapter.activeSceneId === sceneId }"
                  @click="openScene(chapter, sceneId)">{{ studio.scenes.find(item => item.sceneId === sceneId)?.title ?? '场景' }}</button>
              </div>
            </template>
          </template>
        </nav>
        <button v-if="studio.queuedCount || (studio.batch && ['running','paused'].includes(studio.batch.state))" type="button" class="flat-button queue-control" @click="batchPage">
          <Zap :size="14" /> {{ studio.batch?.state === 'paused' ? '后台连写已暂停' : studio.queuedCount + ' 项生成任务' }}
        </button>
        <details v-if="studio.archived.length" class="archive-list">
          <summary>已归档 · {{ studio.archived.length }}</summary>
          <button v-for="chapter in studio.archived" :key="chapter.chapterId" type="button" class="flat-button" @click="studio.restoreChapter(chapter.chapterId)">
            <ArrowLeft :size="13" /> 恢复 {{ chapter.title }}
          </button>
        </details>
        <div class="sidebar-footer"><span class="save-status"></span><span title="工程数据保存在本机">{{ studio.project.path.split(/[\\/]/).at(-1) }}</span></div>
      </aside>

      <section v-if="studio.chapter" class="writing-pane" aria-label="正文编辑器">
        <div class="editor-toolbar">
          <span>正文</span>
          <div>
            <button type="button" class="icon-button" title="上移章节" aria-label="上移章节" :disabled="studio.chapter.chapterOrder <= 1" @click="studio.moveChapter(-1)"><ArrowUp :size="15" /></button>
            <button type="button" class="icon-button" title="下移章节" aria-label="下移章节" :disabled="studio.chapter.chapterOrder >= studio.chapters.length" @click="studio.moveChapter(1)"><ArrowDown :size="15" /></button>
            <button type="button" class="flat-button subaction" @click="archive">归档</button>
            <button type="button" class="secondary-button save-button" :disabled="studio.busy || !studio.dirty" @click="chooseRevision">保存修订</button>
          </div>
        </div>
        <div class="paper">
          <p class="micro">第 {{ studio.chapter.chapterOrder }} 章</p>
          <input :key="studio.chapter.chapterId + studio.chapter.title" :value="studio.chapter.title"
            class="chapter-title" aria-label="章节标题" maxlength="120" @blur="renameChapter" @keydown.enter="($event.target as HTMLInputElement).blur()">
          <div v-if="checks.length" class="manuscript checked-manuscript" aria-label="带一致性告警的正文">
            <template v-for="(part, index) in checks" :key="index"><button v-if="part.issue" type="button" class="redline"
              :title="part.issue.message" @click="locate(studio.chapterId, part.issue.anchor.quote, part.issue.anchor.startOffset)">{{ part.content }}</button><template v-else>{{ part.content }}</template></template>
          </div>
          <textarea v-else ref="manuscript" v-model="studio.text" class="manuscript" spellcheck="false" aria-label="正文" placeholder="从一个动作、一个地点，或一句对白写起……"
            @select="readSelection" @mouseup="readSelection" @keyup="readSelection"></textarea>
          <div v-if="selection.end > selection.start" class="selection-actions" aria-label="选中文字的操作">
            <button class="secondary-button" type="button" @click="panel = 'editing'; showInspector = true">微调选区</button>
            <button class="secondary-button" type="button" :disabled="studio.busy || studio.dirty"
              @click="panel = 'editing'; showInspector = true; studio.requestCritic(selection.start, selection.end)">请主编挑刺</button>
          </div>
        </div>
        <footer class="editor-footer">
          <div v-if="recovered && studio.currentTask" class="recovery recovery-bar">
            <strong><CircleAlert :size="15" /> {{ states[studio.currentTask.state] }}</strong>
            <p>{{ studio.writingIssues[studio.currentTask.taskId] ?? '已收到的正文可继续使用。' }}</p>
            <button v-if="studio.currentTask.partialOutput" type="button" class="flat-button" @click="recover(studio.currentTask, 'keep_draft')">保留草稿</button>
            <button v-if="studio.currentTask.partialOutput" type="button" class="flat-button" @click="recover(studio.currentTask, 'continue_from_draft')">从断点续写</button>
            <button type="button" class="flat-button" @click="recover(studio.currentTask, 'rerun_new_task')">作废重跑</button>
          </div>
          <div v-if="showInspiration" class="inspiration-panel">
            <div class="task-meta"><strong>下一拍灵感</strong><button class="icon-button" type="button" aria-label="关闭灵感" @click="showInspiration = false"><X :size="15" /></button></div>
            <p v-if="studio.inspiration && activeTask(studio.inspiration.state)" class="muted">正在找可推进情节的行动。</p>
            <button v-for="idea in inspirationIdeas" :key="idea" class="memory-row" type="button"
              :disabled="studio.inspiration?.state !== 'completed'" @click="studio.draftHint = idea; showInspiration = false">{{ idea }}</button>
            <p v-if="studio.inspiration && !activeTask(studio.inspiration.state) && studio.inspiration.state !== 'completed'" class="pending-text">
              {{ studio.writingIssues[studio.inspiration.taskId] ?? '本次未生成可用灵感。' }}</p>
          </div>
          <div class="editor-meter"><span>{{ studio.currentCount }} 字</span><div class="meter" role="progressbar" :aria-valuenow="chapterProgress" aria-label="章节目标进度"><span :style="{ width: chapterProgress + '%' }"></span></div>
            <span>{{ studio.dirty ? '编辑缓冲' : studio.chapter.currentRevisionId ? '已保存' : '未开始' }}</span></div>
          <div class="write-bar">
            <input v-model="studio.draftHint" aria-label="续写思路" placeholder="输入这一拍的思路，或留空续写" @keydown.enter="generate">
            <button v-if="studio.currentActive && studio.currentActive.state !== 'queued'" class="secondary-button" type="button" :disabled="studio.busy" @click="studio.cancel(studio.currentActive)">
              <Square :size="15" /> 暂停
            </button>
            <button v-else class="primary-button" type="button" :disabled="studio.busy || !!studio.currentActive" @click="generate"><Sparkles :size="16" /> 一键续写</button>
            <button class="secondary-button" type="button" :disabled="studio.busy || !!(studio.inspiration && activeTask(studio.inspiration.state))"
              @click="askInspiration"><Dices :size="16" /> 灵感</button>
            <button class="secondary-button" type="button" @click="batchPage"><Zap :size="16" /> 托管</button>
          </div>
        </footer>
      </section>
      <section v-else class="writing-pane no-chapter">
        <BookOpen :size="31" /><h2>还没有章节</h2>
        <button type="button" class="primary-button" @click="studio.addChapter('')"><Plus :size="16" /> 创建第一章</button>
      </section>

      <aside v-if="showInspector" class="inspector" aria-label="草稿与修订">
        <nav class="inspector-tabs">
          <button :class="{ selected: panel === 'draft' }" type="button" @click="panel = 'draft'">续写草稿</button>
          <button :class="{ selected: panel === 'history' }" type="button" @click="panel = 'history'"><Clock3 :size="14" /> 版本历史</button>
          <button :class="{ selected: panel === 'review' }" type="button" @click="panel = 'review'">审查<span v-if="reviewCount" class="pending-count">{{ reviewCount }}</span></button>
          <button :class="{ selected: panel === 'editing' }" type="button" @click="panel = 'editing'">编辑</button>
        </nav>
        <template v-if="panel === 'draft'">
          <div class="model-line">
            <span>主笔</span>
            <select v-if="studio.profiles.length" v-model="studio.profileId" aria-label="续写模型">
              <option v-for="profile in studio.profiles" :key="profile.id" :value="profile.id">{{ profile.name }}</option>
            </select>
            <button v-else type="button" class="flat-button" @click="model">配置模型</button>
          </div>
          <SceneControls />
          <div v-if="studio.currentTask" class="task-card">
            <div class="task-meta"><span class="state-label" :class="{ amber: recovered }">{{ states[studio.currentTask.state] }}</span>
              <span>{{ countHan(studio.currentTask.partialOutput) }} 字</span></div>
            <select v-if="studio.chapterTasks.length > 1" v-model="studio.taskId" class="task-select" aria-label="选择历史任务">
              <option disabled value="">最近的任务</option>
              <option v-for="task in studio.chapterTasks" :key="task.taskId" :value="task.taskId">{{ date.format(new Date(task.frozenContext.capturedAt)) }} · {{ states[task.state] }}</option>
            </select>
            <WritingContext :key="studio.currentTask.taskId" :context="studio.currentTask.frozenContext" @open="locate" />
            <div v-if="studio.currentTask.state === 'queued'" class="queued-action">
              <p>此任务尚未送达模型。</p>
              <button type="button" class="primary-button" :disabled="studio.busy" @click="studio.startQueued(studio.currentTask)">开始已排队任务</button>
              <button type="button" class="flat-button" @click="studio.cancel(studio.currentTask)">取消</button>
            </div>
            <p v-if="studio.currentTask.frozenContext.carryDraft" class="carry-label">从已保留的草稿续写</p>
            <div class="draft-paper" :class="{ streaming: activeTask(studio.currentTask.state) }" aria-label="AI 草稿">
              {{ studio.currentTask.partialOutput || (activeTask(studio.currentTask.state) ? '等待模型输出…' : '本次无正文。') }}
            </div>
            <div v-if="studio.currentTask.partialOutput && !activeTask(studio.currentTask.state)" class="draft-actions">
              <button v-if="studio.currentTask.state === 'completed'" class="primary-button" type="button" :disabled="studio.busy || !!studio.writingIssues[studio.currentTask.taskId]
                || studio.currentTask.baseRevisionId !== studio.chapter?.currentRevisionId" @click="studio.acceptDraft(studio.currentTask)">采纳到正文</button>
              <button class="icon-button" type="button" aria-label="复制草稿" @click="copyDraft(studio.currentTask)"><Copy :size="16" /></button>
            </div>
          </div>
          <div v-else class="inspector-empty"><Sparkles :size="20" /><p>续写结果会先出现在这里。</p></div>
        </template>
        <div v-else-if="panel === 'history'" class="history-list">
          <button v-for="version in studio.history" :key="version.revisionId" class="history-row" type="button" @click="studio.restoreRevision(version)">
            <strong>{{ version.revisionId === studio.chapter?.currentRevisionId ? '当前正式修订' : version.saveKind === 'ai_draft' ? '保留草稿' : '正式修订' }}</strong>
            <span>{{ date.format(new Date(version.createdAt)) }} · {{ version.cjkCharCount }} 字</span>
          </button>
          <div v-if="!studio.history.length" class="inspector-empty"><Clock3 :size="20" /><p>保存修订后会显示在这里。</p></div>
        </div>
        <EditingTools v-else-if="panel === 'editing'" :selection="selection" @locate="locate" />
        <div v-else class="review-panel">
          <div class="task-meta"><span>记忆 · {{ sync[studio.chapter?.memorySyncStatus ?? 'pending'] }}</span>
            <button type="button" class="secondary-button" :disabled="studio.busy || !studio.chapter || !studio.text.trim()"
              @click="studio.completeChapter()">完成本章</button></div>
          <article v-for="finding in studio.chapterFindings" :key="finding.anchor.startOffset + finding.message" class="review-finding"
            :class="{ severe: finding.severity === 'severe' }">
            <strong>{{ finding.message }}</strong>
            <button type="button" class="memory-row" @click="locate(studio.chapterId, finding.anchor.quote, finding.anchor.startOffset)">{{ finding.anchor.quote }}</button>
            <p v-if="finding.suggestion">{{ finding.suggestion }}</p>
            <div v-if="finding.severity === 'severe'" class="memory-actions">
              <button class="flat-button" type="button" @click="locate(studio.chapterId, finding.anchor.quote, finding.anchor.startOffset)">手动修改</button>
              <button class="flat-button" type="button" :disabled="studio.busy || studio.dirty" @click="rewriteFinding(finding.anchor.quote, finding.anchor.startOffset)">局部重写</button>
            </div>
          </article>
          <p v-if="studio.chapterAudits.some(task => activeTask(task.state))" class="muted">一致性审查正在运行。</p>
          <p v-else-if="studio.chapterAudits.some(task => task.state === 'completed') && !studio.chapterFindings.length" class="muted">本次审查没有可定位的告警。</p>
          <div v-for="task in studio.chapterAnalysis.filter(item => item.state === 'queued')" :key="task.taskId" class="memory-actions">
            <button type="button" class="secondary-button" :disabled="studio.busy" @click="studio.startQueued(task)">发送{{ task.taskType === 'consistency_check' ? '审查' : '提取' }}</button>
            <button type="button" class="flat-button" :disabled="studio.busy" @click="studio.cancel(task)">取消</button>
          </div>
          <p v-for="task in studio.chapterAnalysis.filter(item => studio.writingIssues[item.taskId])" :key="task.taskId" class="pending-text">{{ studio.writingIssues[task.taskId] }}</p>
          <button v-for="fact in studio.memory.events.filter(item => item.chapterId === studio.chapterId && item.current && item.approval === 'pending')"
            :key="fact.eventId" type="button" class="memory-row" @click="reviewEventId = fact.eventId; view = 'memory'"><strong class="pending-text">待确认</strong>{{ fact.summary }}</button>
        </div>
      </aside>
    </main>

    <div v-if="studio.error" class="toast error-toast" role="alert"><CircleAlert :size="18" /><span>{{ studio.error }}</span>
      <button type="button" class="icon-button" aria-label="关闭提示" @click="studio.error = ''"><X :size="16" /></button>
    </div>
    <div v-else-if="studio.notice" class="toast" role="status">{{ studio.notice }}
      <button v-if="studio.autoFacts.length" type="button" class="flat-button" @click="studio.undoAutomaticMemory()">撤销</button></div>
    <dialog ref="revisionDialog" class="diff-dialog" aria-label="保存修订">
      <form class="memory-form" @submit.prevent="submitRevision">
        <h2>保存修订</h2>
        <label>修改级别<select v-model="editLevel">
          <option value="surface">字词与排版</option><option value="semantic">情节与含义</option><option value="canon">设定变更</option>
        </select></label>
        <label v-if="editLevel !== 'surface'">后文检索词（可选）
          <input v-model="impactTerms" placeholder="例如：铜匙、周宁">
        </label>
        <template v-if="editLevel === 'canon'">
          <label>要更新的权威设定
            <select v-model="canonTarget" required @change="selectCanonTarget">
              <option value="">选择设定</option>
              <option v-for="item in studio.memory.canon" :key="item.entryId" :value="item.entryId">{{ item.name }}</option>
            </select>
          </label>
          <p v-if="!studio.memory.canon.length" class="muted">先在全景库的记忆库新建权威设定。</p>
          <label v-if="canonTarget">更新后的设定<textarea v-model="canonBody" rows="4" required></textarea></label>
          <button type="button" class="secondary-button" :disabled="!canonTarget || previewingCanon" @click="previewCanonRevision">预览影响范围</button>
          <template v-if="revisionCanon">
            <CanonImpact :impact="revisionCanon.impact" />
            <label class="check-line"><input v-model="confirmed" type="checkbox" required> 确认同时保存正文和权威设定</label>
          </template>
        </template>
        <p v-if="studio.error" class="pending-text" role="alert">{{ studio.error }}</p>
        <div class="memory-actions"><button type="submit" class="primary-button" :disabled="studio.busy || (editLevel === 'canon' && (!revisionCanon || !confirmed))">确认保存</button>
          <button class="flat-button" type="button" @click="revisionDialog?.close()">取消</button></div>
      </form>
    </dialog>
    <dialog ref="canonDialog" class="diff-dialog" aria-label="权威设定影响范围" @close="studio.canonProposal = null">
      <form v-if="studio.canonProposal" class="memory-form" @submit.prevent="studio.confirmCanonChange()">
        <h2>更新「{{ studio.canonProposal.entry.name }}」的影响范围</h2>
        <CanonImpact :impact="studio.canonProposal.impact" />
        <p v-if="studio.error" class="pending-text" role="alert">{{ studio.error }}</p>
        <div class="memory-actions">
          <button type="submit" class="primary-button" :disabled="studio.busy">确认更新权威设定</button>
          <button class="flat-button" type="button" @click="canonDialog?.close()">取消</button>
        </div>
      </form>
    </dialog>
    <ModelSettings v-if="settings" @close="settings = false" />
  </div>
</template>

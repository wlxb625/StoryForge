<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { isTauri } from '@tauri-apps/api/core'
import {
  ArrowDown, ArrowLeft, ArrowRight, ArrowUp, BookOpen, ChevronDown, CircleAlert,
  Beaker, Clock3, Copy, FilePlus2, FolderOpen, Library, PanelRight, PenLine, Plus, Settings2, Sparkles, Square, X,
} from 'lucide-vue-next'
import ModelSettings from './components/ModelSettings.vue'
import SceneControls from './components/SceneControls.vue'
import PacksView from './views/PacksView.vue'
import LabView from './views/LabView.vue'
import { activeTask, type GenerationTask, type TaskState } from './core/contracts/types'
import { countHan } from './core/quality/hardGate'
import { useStudio } from './stores/studio'

const studio = useStudio()
const settings = ref(false)
const title = ref('')
const creatingBook = ref(false)
const chapterForm = ref(false)
const newChapterTitle = ref('')
const panel = ref<'draft' | 'history'>('draft')
const showInspector = ref(true)
const view = ref<'studio' | 'packs' | 'lab'>('studio')
watch(() => studio.project?.projectId, () => { view.value = 'studio' })
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

function keyboard(event: KeyboardEvent) {
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 's' && studio.project) {
    event.preventDefault()
    void studio.saveCurrent()
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
        <span v-if="studio.queuedCount" class="queue-indicator"><span></span>{{ studio.queuedCount }} 项生成任务</span>
      </div>
      <nav v-if="studio.project" class="workspace-nav" aria-label="工作台导航">
        <button class="flat-button" :class="{ selected: view === 'studio' }" type="button" aria-label="写作台" @click="view = 'studio'"><PenLine :size="16" /><span>写作台</span></button>
        <button class="flat-button" :class="{ selected: view === 'packs' }" type="button" aria-label="卡带与配方" @click="view = 'packs'"><Library :size="16" /><span>配方</span></button>
        <button class="flat-button" :class="{ selected: view === 'lab' }" type="button" aria-label="实验室" @click="view = 'lab'"><Beaker :size="16" /><span>实验室</span></button>
      </nav>
      <div class="header-actions">
        <button v-if="studio.project" type="button" class="flat-button desktop-only" :disabled="studio.busy" @click="studio.openWorkspace()"><FolderOpen :size="16" /> 打开工程</button>
        <button type="button" class="flat-button" aria-label="模型设置" @click="model"><Settings2 :size="17" /><span>{{ studio.selectedProfile?.name ?? '模型设置' }}</span></button>
        <button v-if="studio.project && view === 'studio'" class="icon-button" type="button" :aria-pressed="showInspector" aria-label="切换右侧面板" @click="showInspector = !showInspector"><PanelRight :size="18" /></button>
      </div>
    </header>

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
    <LabView v-else-if="view === 'lab'" />
    <main v-else class="studio-layout" :class="{ compact: !showInspector }">
      <aside class="chapter-pane" aria-label="章节目录">
        <div class="aside-head"><strong>章节</strong><button type="button" class="icon-button" aria-label="添加章节" @click="chapterForm = !chapterForm"><Plus :size="18" /></button></div>
        <form v-if="chapterForm" class="chapter-form" @submit.prevent="createChapter">
          <input v-model="newChapterTitle" maxlength="120" placeholder="新章标题" aria-label="新章标题">
          <button class="primary-button" type="submit" :disabled="studio.busy">创建</button>
        </form>
        <nav class="chapter-list">
          <button v-for="chapter in studio.chapters" :key="chapter.chapterId" type="button" :class="{ selected: studio.chapterId === chapter.chapterId }"
            :aria-current="studio.chapterId === chapter.chapterId ? 'page' : undefined" @click="studio.openChapter(chapter.chapterId)">
            <span class="chapter-no">{{ String(chapter.chapterOrder).padStart(2, '0') }}</span>
            <span>{{ chapter.title }}</span><span v-if="chapter.currentRevisionId" class="chapter-dot"></span>
          </button>
        </nav>
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
            <button type="button" class="secondary-button save-button" :disabled="studio.busy || !studio.dirty" @click="studio.saveCurrent()">保存修订</button>
          </div>
        </div>
        <div class="paper">
          <p class="micro">第 {{ studio.chapter.chapterOrder }} 章</p>
          <input :key="studio.chapter.chapterId + studio.chapter.title" :value="studio.chapter.title"
            class="chapter-title" aria-label="章节标题" maxlength="120" @blur="renameChapter" @keydown.enter="($event.target as HTMLInputElement).blur()">
          <textarea v-model="studio.text" class="manuscript" spellcheck="false" aria-label="正文" placeholder="从一个动作、一个地点，或一句对白写起……"></textarea>
        </div>
        <footer class="editor-footer">
          <div class="editor-meter"><span>{{ studio.currentCount }} 字</span><div class="meter" role="progressbar" :aria-valuenow="chapterProgress" aria-label="章节目标进度"><span :style="{ width: chapterProgress + '%' }"></span></div>
            <span>{{ studio.dirty ? '编辑缓冲' : studio.chapter.currentRevisionId ? '已保存' : '未开始' }}</span></div>
          <div class="write-bar">
            <input v-model="studio.draftHint" aria-label="续写思路" placeholder="输入这一拍的思路，或留空续写" @keydown.enter="generate">
            <button v-if="studio.currentActive && studio.currentActive.state !== 'queued'" class="secondary-button" type="button" :disabled="studio.busy" @click="studio.cancel(studio.currentActive)">
              <Square :size="15" /> 暂停
            </button>
            <button v-else class="primary-button" type="button" :disabled="studio.busy || !!studio.currentActive" @click="generate"><Sparkles :size="16" /> 一键续写</button>
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
            <div v-if="recovered" class="recovery">
              <strong><CircleAlert :size="15" /> {{ states[studio.currentTask.state] }}</strong>
              <p>{{ studio.writingIssues[studio.currentTask.taskId] ?? '已收到的正文可继续使用。' }}</p>
              <button v-if="studio.currentTask.partialOutput" type="button" class="flat-button" @click="recover(studio.currentTask, 'keep_draft')">保留草稿</button>
              <button v-if="studio.currentTask.partialOutput" type="button" class="flat-button" @click="recover(studio.currentTask, 'continue_from_draft')">从断点续写</button>
              <button type="button" class="flat-button" @click="recover(studio.currentTask, 'rerun_new_task')">作废重跑</button>
            </div>
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
        <div v-else class="history-list">
          <button v-for="version in studio.history" :key="version.revisionId" class="history-row" type="button" @click="studio.restoreRevision(version)">
            <strong>{{ version.revisionId === studio.chapter?.currentRevisionId ? '当前正式修订' : version.saveKind === 'ai_draft' ? '保留草稿' : '正式修订' }}</strong>
            <span>{{ date.format(new Date(version.createdAt)) }} · {{ version.cjkCharCount }} 字</span>
          </button>
          <div v-if="!studio.history.length" class="inspector-empty"><Clock3 :size="20" /><p>保存修订后会显示在这里。</p></div>
        </div>
      </aside>
    </main>

    <div v-if="studio.error" class="toast error-toast" role="alert"><CircleAlert :size="18" /><span>{{ studio.error }}</span>
      <button type="button" class="icon-button" aria-label="关闭提示" @click="studio.error = ''"><X :size="16" /></button>
    </div>
    <div v-else-if="studio.notice" class="toast" role="status">{{ studio.notice }}</div>
    <ModelSettings v-if="settings" @close="settings = false" />
  </div>
</template>

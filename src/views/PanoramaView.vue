<script setup lang="ts">
import { computed, ref, toRaw, watch } from 'vue'
import MarkdownIt from 'markdown-it'
import { Binoculars, Download, Pause, Play, Square } from 'lucide-vue-next'
import type { Canon } from '../core/narrative/memory'
import { eventTypes } from '../core/narrative/memory'
import { useStudio } from '../stores/studio'
import { desktop } from '../core/db/desktop'
import { auditFindings } from '../core/narrative/memory'
import type { Batch, BatchDraft, BatchSummary } from '../core/tasks/batch'
import BatchFlow from '../components/BatchFlow.vue'

const options = defineProps<{ openBatch?: boolean }>()
const emit = defineEmits<{ open: [chapterId: string, quote: string, offset?: number]; review: [chapterId: string]; memory: [] }>()
const studio = useStudio()
const markdown = new MarkdownIt({ html: false, linkify: false }).disable(['link','image','autolink'])
const tabs = { people: '人物状态', props: '道具账本', timeline: '时间线', relations: '人物关系', hooks: '伏笔与回旋', batch: '托管控制台' } as const
const tab = ref<keyof typeof tabs>(options.openBatch ? 'batch' : 'people')
watch(() => options.openBatch, open => { if (open) tab.value = 'batch' })
const people = computed(() => studio.memory.canon.filter(item => item.category === 'character'))
const character = ref<Canon>()
const draft = ref('')
const cardHtml = computed(() => markdown.render(draft.value))
const eventList = computed(() => studio.memory.events.filter(event => event.current && ['auto_accepted','author_accepted'].includes(event.approval)))
const worldOrder = ref(false)
const temporalLabel = new Intl.Collator('zh-CN', { numeric: true })
const timeline = computed(() => worldOrder.value
  ? [...eventList.value].sort((a, b) => Number(!a.time) - Number(!b.time)
    || temporalLabel.compare(a.time, b.time) || a.chapterOrder - b.chapterOrder)
  : eventList.value)
const selectedProp = ref('')
const props = computed(() => studio.memory.canon.filter(item => item.category === 'prop'))
const selectedNames = computed(() => {
  const entry = props.value.find(item => item.entryId === selectedProp.value)
  return entry ? [entry.name, ...entry.aliases] : []
})
const propHistory = computed(() => eventList.value.filter(event => event.eventType === 'prop_transfer' &&
  (!selectedProp.value || event.subjects.includes(selectedProp.value)
    || selectedNames.value.some(name => event.summary.includes(name)))))
const relationFacts = computed(() => eventList.value.filter(event => event.eventType === 'relation_shift'))
const center = ref('')
const graphPeople = computed(() => {
  const named = new Set(relationFacts.value.filter(event => !center.value || event.subjects.includes(center.value)).flatMap(event => event.subjects))
  return people.value.filter(person => named.has(person.entryId)).slice(0, 18)
})
const nodes = computed(() => graphPeople.value.map((person, index, all) => {
  const angle = index * Math.PI * 2 / all.length - Math.PI / 2
  return { person, x: 330 + Math.cos(angle) * 254, y: 200 + Math.sin(angle) * 145 }
}))
const edges = computed(() => {
  const seen = new Set<string>()
  return [...relationFacts.value].reverse().flatMap(event => {
    if (center.value && !event.subjects.includes(center.value)) return []
    const members = nodes.value.filter(node => event.subjects.includes(node.person.entryId))
    return members.flatMap((first, index) => members.slice(index + 1).flatMap(second => {
      const key = [first.person.entryId, second.person.entryId].sort().join('|')
      if (seen.has(key)) return []
      seen.add(key)
      return [{ key, first, second, event }]
    }))
  })
})
const hookStatus = { open: '待回收', reminded: '已提醒', paid_off: '已回收', deprecated: '已废弃' } as const
const hookType = { foreshadow: '伏笔', gag_hook: '包袱回旋', subtext_cue: '潜台词' }
const batchStates = { running: '正在连写', paused: '已暂停', stopped: '已结束', completed: '已完成' }
const savedRuns = ref<BatchSummary[]>([])
const historyId = ref('')
const historicalRun = ref<Batch>()
const dateTime = new Intl.DateTimeFormat('zh-CN', { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' })
watch(() => [tab.value, studio.batch?.batchId, studio.batch?.state], async ([tab], _, onCleanup) => {
  if (tab !== 'batch') return
  let stale = false
  onCleanup(() => { stale = true })
  try {
    const rows = await desktop.batchHistory()
    if (!stale) savedRuns.value = rows
  } catch { if (!stale) studio.error = '无法读取托管历史。' }
}, { immediate: true })
watch(historyId, async (id, _, onCleanup) => {
  historicalRun.value = undefined
  if (!id) return
  let stale = false
  onCleanup(() => { stale = true })
  try {
    const run = await desktop.namedBatch(id)
    if (!stale) historicalRun.value = run
  } catch { if (!stale) studio.error = '无法读取所选批次。' }
})
const stagedDraft = ref<BatchDraft | null>(null)
watch(() => [studio.batch?.batchId, studio.batch?.draftTaskId] as const, async ([batchId, draftId]) => {
  const draft = batchId && draftId ? await desktop.previewBatchDraft(batchId).catch(() => null) : null
  if (studio.batch?.draftTaskId === draftId) stagedDraft.value = draft
}, { immediate: true })
const stagedFindings = computed(() => {
  const draft = stagedDraft.value
  if (!draft) return []
  const review = studio.tasks.find(task => task.state === 'completed' && task.taskType === 'consistency_check'
    && task.frozenContext.stagedDraftId === draft.taskId)
  const local = review?.frozenContext.viewpointAlerts ?? []
  try { return review ? [...local, ...auditFindings(review.partialOutput, draft.content)] : [] }
  catch { return local }
})
const chapterIds = ref<string[]>(studio.chapterId ? [studio.chapterId] : [])
const budget = ref(studio.batch?.budgetUsd ?? 0.02)
const instruction = ref('')
const mode = ref<Batch['mode']>('assisted')
const picks = computed(() => studio.profiles.filter(profile => profile.rates && (!profile.needsKey || profile.hasKey)))
const writer = ref(picks.value.find(profile => profile.id === studio.profileId)?.id ?? picks.value[0]?.id ?? '')
const audit = ref(studio.memory.roles.auditProfileId ?? writer.value)
const extractor = ref(studio.memory.roles.extractionProfileId ?? writer.value)
const live = computed(() => studio.batch && ['running','paused'].includes(studio.batch.state))
const confirmed = ref(false)
const exportFormat = ref<'story' | 'docx' | 'epub'>('story')
const usd = (value: number) => '$' + value.toFixed(6)

const queueTasks = computed(() => {
  const drafts = studio.tasks.filter(task => task.frozenContext.batchId === studio.batch?.batchId
    && task.taskType === 'batch_write' && task.partialOutput)
  return drafts.filter((task, index) => ['failed','cancelled','outcome_unknown','interrupted'].includes(task.state)
    || (index === 0 && studio.batch?.state === 'paused' && task.state === 'completed')).slice(0, 12)
})

function edit(person: Canon) {
  character.value = structuredClone(toRaw(person))
  draft.value = person.body
}
async function save() {
  if (!character.value) return
  await studio.saveCanon({ ...character.value, body: draft.value })
  if (!studio.error && !studio.canonProposal) {
    const updated = studio.memory.canon.find(item => item.entryId === character.value?.entryId)
    if (updated) edit(updated)
  }
}
watch(() => people.value.find(item => item.entryId === character.value?.entryId)?.version, version => {
  const updated = people.value.find(item => item.entryId === character.value?.entryId)
  if (updated && version !== character.value?.version) edit(updated)
})
function start() {
  if (!confirmed.value) return
  void studio.startBatch({ chapterIds: chapterIds.value, budgetUsd: budget.value, instruction: instruction.value,
    writerProfile: writer.value, auditProfile: audit.value, extractionProfile: extractor.value, mode: mode.value })
}
</script>

<template>
  <main class="workspace-page panorama-page">
    <header class="workspace-intro panorama-heading">
      <h1><Binoculars :size="26" /> 全景</h1>
      <div class="memory-actions">
        <button class="secondary-button" type="button" @click="emit('memory')">记忆库 <span v-if="studio.memoryPending">{{ studio.memoryPending }}</span></button>
        <select v-model="exportFormat" aria-label="导出格式">
          <option value="story">工程快照 .story</option><option value="docx">成稿 .docx</option><option value="epub">电子书 .epub</option>
        </select>
        <button class="secondary-button" type="button" :disabled="studio.busy" @click="studio.exportBook(exportFormat)"><Download :size="16" /> 导出</button>
      </div>
    </header>
    <nav class="workspace-subnav" aria-label="全景导航">
      <button v-for="(label, id) in tabs" :key="id" type="button" class="flat-button" :class="{ selected: tab === id }" @click="tab = id">{{ label }}</button>
    </nav>
    <div v-if="tab === 'people'" class="memory-grid">
      <section class="memory-list" aria-label="人物">
        <button v-for="person in people" :key="person.entryId" class="memory-row" type="button"
          :class="{ selected: character?.entryId === person.entryId }" @click="edit(person)"><strong>{{ person.name }}</strong>
          <small v-if="person.aliases.length">{{ person.aliases.join('、') }}</small></button>
        <p v-if="!people.length" class="muted">人物从记忆库的权威设定读取。</p>
      </section>
      <form v-if="character" class="workspace-field memory-form" @submit.prevent="save">
        <h2>{{ character.name }}</h2>
        <label>人物状态 Markdown<textarea v-model="draft" rows="7"></textarea></label>
        <label class="check-line"><input v-model="character.locked" type="checkbox"> 锁定为作者权威设定</label>
        <button type="submit" class="primary-button" :disabled="studio.busy || (draft === character.body
          && character.locked === people.find(person => person.entryId === character?.entryId)?.locked)">保存人物状态</button>
        <article v-if="draft" class="rendered-card" v-html="cardHtml"></article>
      </form>
    </div>
    <section v-else-if="tab === 'props' || tab === 'timeline'" class="workspace-field memory-form panorama-records">
      <label v-if="tab === 'props'">筛选道具<select v-model="selectedProp"><option value="">全部流转</option>
        <option v-for="prop in props" :key="prop.entryId" :value="prop.entryId">{{ prop.name }}</option></select></label>
      <label v-else class="check-line"><input v-model="worldOrder" type="checkbox"> 按故事时间标签排序</label>
      <ol class="ledger">
        <li v-for="event in tab === 'props' ? propHistory : timeline" :key="event.eventId">
          <span class="ledger-marker"></span><small class="muted">第 {{ event.chapterOrder }} 章<span v-if="event.time"> · {{ event.time }}</span> · {{ eventTypes[event.eventType] }}</small>
          <p>{{ event.summary }}</p>
          <button type="button" class="flat-button" @click="emit('open', event.chapterId, event.anchor.quote, event.anchor.startOffset)">查看原文</button>
        </li>
      </ol>
      <p v-if="!(tab === 'props' ? propHistory : eventList).length" class="muted">这里显示已确认且可定位到正文的事件。</p>
    </section>
    <section v-else-if="tab === 'relations'" class="workspace-field memory-form relationship-panel">
      <label>人物范围<select v-model="center"><option value="">所有已记录关系</option>
        <option v-for="person in people" :key="person.entryId" :value="person.entryId">{{ person.name }}</option></select></label>
      <svg v-if="edges.length" class="relationship-graph" viewBox="0 0 660 400" role="img" aria-label="已确认的人物关系图">
        <line v-for="edge in edges" :key="edge.key" :x1="edge.first.x" :y1="edge.first.y" :x2="edge.second.x" :y2="edge.second.y"><title>{{ edge.event.summary }}</title></line>
        <g v-for="node in nodes" :key="node.person.entryId">
          <circle :cx="node.x" :cy="node.y" r="22"><title>{{ node.person.name }}</title></circle>
          <text :x="node.x" :y="node.y + 4" text-anchor="middle">{{ node.person.name }}</text>
        </g>
      </svg>
      <p v-else class="muted">确认包含至少两位已登记人物的关系事件后，连线会出现在这里。</p>
      <button v-for="event in relationFacts.filter(item => !center || item.subjects.includes(center))" :key="event.eventId"
        type="button" class="memory-row" @click="emit('open', event.chapterId, event.anchor.quote, event.anchor.startOffset)">{{ event.summary }}</button>
    </section>
    <div v-else-if="tab === 'hooks'" class="hook-board">
      <section v-for="(label, state) in hookStatus" :key="state" class="workspace-field memory-form">
        <h2>{{ label }}</h2>
        <p v-if="!studio.memory.notes.some(note => note.status === state)" class="muted">暂无记录</p>
        <article v-for="note in studio.memory.notes.filter(item => item.status === state)" :key="note.noteId" class="hook-card">
          <small>{{ hookType[note.noteType] }} · 第 {{ note.chapterOrder }} 章</small><p>{{ note.body }}</p>
          <label>更新进度<select :value="note.status" @change="studio.saveNote({ ...note, status: ($event.target as HTMLSelectElement).value as typeof note.status })">
            <option v-for="(option, value) in hookStatus" :key="value" :value="value">{{ option }}</option>
          </select></label>
        </article>
      </section>
    </div>
    <div v-else class="hosting-grid">
      <section v-if="studio.batch" class="workspace-field memory-form hosting-flow">
        <label v-if="savedRuns.length > 1">托管批次
          <select v-model="historyId">
            <option value="">当前或最近一次</option>
            <option v-for="run in savedRuns.filter(item => item.batchId !== studio.batch?.batchId)" :key="run.batchId" :value="run.batchId">
              {{ dateTime.format(new Date(run.createdAt)) }} · {{ run.mode === 'assisted' ? '辅助托管' : '全自动托管' }} · {{ batchStates[run.state] }}
            </option>
          </select>
        </label>
        <BatchFlow v-if="!historyId || historicalRun" :run="historicalRun ?? studio.batch" @open="(...args) => emit('open', ...args)" />
        <p v-if="historicalRun?.reason" class="muted">{{ historicalRun.reason }}</p>
      </section>
      <section v-if="studio.batch" class="workspace-field memory-form batch-status">
        <h2>{{ batchStates[studio.batch.state] }}</h2>
        <p>{{ studio.batch.cursor }} / {{ studio.batch.chapterIds.length }} 章</p>
        <div class="meter" role="progressbar" aria-label="后台连写进度" :aria-valuenow="studio.batch.cursor" :aria-valuemax="studio.batch.chapterIds.length">
          <span :style="{ width: studio.batch.cursor / studio.batch.chapterIds.length * 100 + '%' }"></span>
        </div>
        <dl><dt>已记账 / 预算</dt><dd>{{ usd(studio.batch.spentUsd) }} / {{ usd(studio.batch.budgetUsd) }}</dd>
          <dt v-if="studio.batch.step">请求预留</dt><dd v-if="studio.batch.step">{{ usd(studio.batch.step.reservedUsd) }}</dd>
          <dt v-if="studio.batch.estimatedSteps">含估算的步骤</dt><dd v-if="studio.batch.estimatedSteps">{{ studio.batch.estimatedSteps }}</dd></dl>
        <p v-if="studio.batch.reason" :class="studio.batch.state === 'paused' ? 'pending-text' : 'muted'">{{ studio.batch.reason }}</p>
        <article v-for="finding in stagedFindings" :key="finding.anchor.startOffset + finding.message"
          class="review-finding" :class="{ severe: finding.severity === 'severe' }">
          <strong>{{ finding.message }}</strong><blockquote>{{ finding.anchor.quote }}</blockquote><p>{{ finding.suggestion }}</p>
        </article>
        <details v-if="stagedDraft" :open="studio.batch.phase === 'approval'">
          <summary>待审查的暂存稿</summary><p class="saved-batch-draft">{{ stagedDraft.appendix }}</p>
        </details>
        <button v-if="live && studio.batch.chapterIds[studio.batch.cursor]" type="button" class="flat-button"
          @click="emit('review', studio.batch.chapterIds[studio.batch.cursor]!)">查看当前章审查</button>
        <button v-if="studio.batch.state === 'paused' && studio.batch.phase === 'advance'" type="button" class="flat-button"
          @click="emit('memory')">进入记忆库确认候选</button>
        <label v-if="studio.batch.state === 'paused'">继续使用的预算（美元）<input v-model.number="budget" type="number" min="0.000001" step="0.000001"></label>
        <div v-if="live" class="memory-actions">
          <button v-if="studio.batch.state === 'running'" class="secondary-button" type="button" @click="studio.controlBatch('pause')"><Pause :size="16" /> 暂停</button>
          <button v-else-if="studio.batch.phase === 'approval'" class="primary-button" type="button"
            @click="studio.controlBatch('approve', budget)"><Play :size="16" /> 采纳本章并继续</button>
          <button v-else class="primary-button" type="button" @click="studio.controlBatch('resume', budget)"><Play :size="16" /> 确认继续</button>
          <button v-if="studio.batch.state === 'paused' && studio.batch.draftTaskId && !studio.batch.step" class="flat-button" type="button"
            @click="studio.controlBatch('reset_draft')">归档暂存稿并重新起草</button>
          <button v-if="studio.batch.state === 'paused' && ['audit','approval','adopt'].includes(studio.batch.phase) && studio.batch.draftTaskId && !studio.batch.step"
            class="flat-button" type="button" @click="studio.controlBatch('reaudit')">重新审查暂存稿</button>
          <button class="flat-button" type="button" @click="studio.controlBatch('stop')"><Square :size="15" /> 结束批次</button>
        </div>
        <details v-for="task in queueTasks" :key="task.taskId"><summary>保留的后台草稿</summary><p class="saved-batch-draft">{{ task.partialOutput }}</p></details>
      </section>
      <form v-if="!live" class="workspace-field memory-form" @submit.prevent="start">
        <h2>新建托管</h2>
        <label>托管方式<select v-model="mode">
          <option value="assisted">辅助托管 · 审查后逐章确认</option>
          <option value="automatic">全自动托管 · 审查通过后自动采纳</option>
        </select></label>
        <fieldset class="audience-fields batch-chapters"><legend>章节</legend>
          <label v-for="chapter in studio.chapters" :key="chapter.chapterId" class="check-line">
            <input v-model="chapterIds" type="checkbox" :value="chapter.chapterId">{{ chapter.chapterOrder }}　{{ chapter.title }}
          </label>
        </fieldset>
        <label>预算上限（美元）<input v-model.number="budget" type="number" min="0.000001" step="0.000001" required></label>
        <label>主笔<select v-model="writer" required><option disabled value="">选择已设置单价的模型</option>
          <option v-for="profile in picks" :key="profile.id" :value="profile.id">{{ profile.name }}</option></select></label>
        <label>一致性审查<select v-model="audit" required><option disabled value="">选择模型</option>
          <option v-for="profile in picks" :key="profile.id" :value="profile.id">{{ profile.name }}</option></select></label>
        <label>证据提取<select v-model="extractor" required><option disabled value="">选择模型</option>
          <option v-for="profile in picks" :key="profile.id" :value="profile.id">{{ profile.name }}</option></select></label>
        <label>写作要求<textarea v-model="instruction" rows="3" maxlength="1200"></textarea></label>
        <label class="check-line"><input v-model="confirmed" type="checkbox" required>{{ mode === 'assisted' ? '按所选范围和预算开始起草，每章由我确认后入稿' : '允许审查通过的正文自动入稿，遇异常暂停' }}</label>
        <button class="primary-button" type="submit" :disabled="studio.busy || !picks.length || !chapterIds.length">启动托管</button>
        <p v-if="!picks.length" class="muted">在模型设置填写单价后即可使用。</p>
      </form>
    </div>
  </main>
</template>

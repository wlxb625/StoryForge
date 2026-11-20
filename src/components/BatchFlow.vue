<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { Check, CirclePause, LoaderCircle } from 'lucide-vue-next'
import type { Batch, BatchActivity } from '../core/tasks/batch'
import type { GenerationTask } from '../core/contracts/types'
import { desktop } from '../core/db/desktop'
import { useStudio } from '../stores/studio'
import WritingContext from './WritingContext.vue'

const props = defineProps<{ run: Batch }>()
const emit = defineEmits<{ open: [chapterId: string, quote: string, offset: number] }>()
const studio = useStudio()
const chosen = ref('')
const chapterId = computed(() => chosen.value || props.run.chapterIds[Math.min(props.run.cursor, props.run.chapterIds.length - 1)])
const activity = ref<BatchActivity[]>([])
const inspected = ref<GenerationTask>()
const inspectedId = ref('')
const error = ref('')
const phaseNames = { write: '起草', audit: '一致性审查', approval: '作者确认', adopt: '正式保存', extract: '提取证据', advance: '同步记忆' } as const
const orderedPhases = computed(() => (Object.keys(phaseNames) as Batch['phase'][]).filter(phase => phase !== 'approval' || props.run.mode === 'assisted'))
const currentIndex = computed(() => chapterId.value ? props.run.chapterIds.indexOf(chapterId.value) : -1)
const chapters = computed(() => new Map([...studio.chapters, ...studio.archived].map(item => [item.chapterId, item])))
const completed = computed(() => currentIndex.value >= 0 && currentIndex.value < props.run.cursor)
const inspectedTask = computed(() => studio.tasks.find(task => task.taskId === inspectedId.value) ?? inspected.value)
const date = new Intl.DateTimeFormat('zh-CN', { hour: '2-digit', minute: '2-digit', second: '2-digit' })

watch(() => props.run.batchId, () => { chosen.value = ''; inspectedId.value = '' })
watch(chapterId, () => { inspectedId.value = ''; activity.value = [] })
watch(() => [props.run.batchId, props.run.phase, props.run.state, props.run.cursor, props.run.step?.taskId, props.run.draftTaskId, chapterId.value],
  async (_, __, onCleanup) => {
    let stale = false
    onCleanup(() => { stale = true })
    if (!chapterId.value) return
    try {
      const rows = await desktop.batchActivity(props.run.batchId, chapterId.value)
      if (!stale) { activity.value = rows; error.value = '' }
    } catch {
      if (!stale) { activity.value = []; error.value = '无法读取这一章的运行记录。' }
    }
  }, { immediate: true })
watch(inspectedId, async (id, _, onCleanup) => {
  let stale = false
  onCleanup(() => { stale = true })
  inspected.value = undefined
  if (!id || studio.tasks.some(task => task.taskId === id)) return
  try {
    const task = await desktop.task(id)
    if (!stale) inspected.value = task
  } catch { if (!stale) error.value = '无法读取这个请求。' }
})

function nodeState(phase: Batch['phase']) {
  const skipped = (phase === 'write' || phase === 'approval' || phase === 'adopt')
    && !activity.value.some(step => step.phase === 'write' && step.state === 'completed' && !step.problem)
  if (completed.value) return skipped ? 'skipped' : 'done'
  if (currentIndex.value !== props.run.cursor) return 'waiting'
  const current = orderedPhases.value.indexOf(props.run.phase)
  const offset = orderedPhases.value.indexOf(phase)
  if (offset < current) return skipped ? 'skipped' : 'done'
  if (offset > current) return 'waiting'
  return props.run.state === 'running' ? 'active' : props.run.state === 'paused' ? 'paused' : 'waiting'
}
const nodeLabels = { done: '已完成', active: '进行中', paused: '已暂停', waiting: '待执行', skipped: '沿用现有正文' }
const requestLabels: Record<string, string> = { queued: '待发送', running: '已连接', streaming: '正在输出', completed: '已返回',
  failed: '失败', cancelled: '已取消', interrupted: '已中断', outcome_unknown: '结果未知' }
</script>

<template>
  <section class="batch-flow" aria-label="托管流程">
    <div class="batch-flow-header">
      <h3>托管流程</h3>
      <select v-model="chosen" aria-label="查看流程章节">
        <option value="">跟随当前进度</option>
        <option v-for="id in run.chapterIds" :key="id" :value="id">{{ chapters.get(id)?.title ?? '已移除章节' }}</option>
      </select>
    </div>
    <p v-if="chapterId" class="muted">{{ chapters.get(chapterId)?.title ?? '已移除章节' }} · {{ run.mode === 'assisted' ? '辅助托管' : '全自动托管' }}</p>
    <ol class="batch-nodes" :style="{ '--node-count': orderedPhases.length }">
      <li v-for="key in orderedPhases" :key="key" :data-state="nodeState(key)" :aria-current="nodeState(key) === 'active' ? 'step' : undefined">
        <span class="batch-node-icon" aria-hidden="true">
          <Check v-if="nodeState(key) === 'done'" :size="14" />
          <LoaderCircle v-else-if="nodeState(key) === 'active'" :size="14" class="running-icon" />
          <CirclePause v-else-if="nodeState(key) === 'paused'" :size="14" />
        </span>
        <strong>{{ phaseNames[key] }}</strong><small>{{ nodeLabels[nodeState(key)] }}</small>
      </li>
    </ol>
    <div v-if="activity.length" class="batch-activity">
      <button v-for="step in activity" :key="step.taskId" type="button" class="memory-row" :aria-pressed="inspectedId === step.taskId"
        @click="inspectedId = inspectedId === step.taskId ? '' : step.taskId">
        <strong>{{ phaseNames[step.phase] }} · {{ step.model || '模型记录缺失' }}</strong>
        <small>{{ date.format(new Date(step.createdAt)) }} · {{ requestLabels[step.state] ?? step.state }}<template v-if="step.costUsd != null"> · ${{ step.costUsd.toFixed(6) }}</template></small>
        <small v-if="step.problem" class="pending-text">{{ step.problem }}</small>
      </button>
    </div>
    <p v-if="error" class="pending-text">{{ error }}</p>
    <article v-if="inspectedTask && inspectedId" class="batch-task-detail">
      <h4>{{ inspectedTask.frozenContext.chapterTitle }} · {{ inspectedTask.frozenContext.model }}</h4>
      <WritingContext :key="inspectedId" :context="inspectedTask.frozenContext" @open="(...args) => emit('open', ...args)" />
      <details :key="inspectedId + '-prompt'"><summary>请求提示词</summary><pre class="plain-text frozen-prompt">{{ inspectedTask.frozenContext.prompt }}</pre></details>
      <details v-if="inspectedTask.partialOutput" :key="inspectedId + '-output'"><summary>模型输出</summary><p class="saved-batch-draft">{{ inspectedTask.partialOutput }}</p></details>
    </article>
  </section>
</template>

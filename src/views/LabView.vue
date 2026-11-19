<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { Beaker, Check, RotateCw } from 'lucide-vue-next'
import { activeTask, type Trace } from '../core/contracts/types'
import { estimate } from '../core/providers/pricing'
import { useStudio } from '../stores/studio'

const studio = useStudio()
const tab = ref<'prompt' | 'usage' | 'compare'>('prompt')
const taskId = ref('')
const selectedTask = computed(() => studio.tasks.find(task => task.taskId === taskId.value) ?? studio.tasks[0])
const override = ref(studio.promptHistory[0]?.instruction ?? '')
const modelA = ref(studio.profiles[0]?.id ?? '')
const modelB = ref(studio.profiles[1]?.id ?? '')
const instruction = ref(studio.draftHint)
const chosenGroup = ref('')
const reveal = ref(false)
const comparisons = computed(() => [...new Set(studio.tasks.flatMap(task => task.frozenContext.comparison?.id ?? []))])
const group = computed(() => chosenGroup.value || comparisons.value[0])
const sides = computed(() => studio.tasks.filter(task => task.frozenContext.comparison?.id === group.value)
  .sort((a, b) => (a.frozenContext.comparison?.label ?? '').localeCompare(b.frozenContext.comparison?.label ?? '')))
const date = new Intl.DateTimeFormat('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' })
const token = (value: number | null) => value == null ? '未知' : value.toLocaleString('zh-CN')

function cost(trace: Trace) {
  const context = studio.tasks.find(task => task.taskId === trace.taskId)?.frozenContext
  const range = estimate(trace, context?.rates)
  if (!range) return '未知'
  if (range[0] === range[1]) return '$' + range[0].toFixed(8)
  return '$' + range[0].toFixed(8) + '–' + range[1].toFixed(8)
}
function tag(taskType: string) { return ({ beat_write: '续写', ab_write: 'A/B', skill_extract: 'Skill 提取' } as Record<string,string>)[taskType] ?? taskType }
function blindProse(text: string) {
  if (reveal.value) return text
  return [...new Set(sides.value.map(task => task.frozenContext.model))]
    .filter(Boolean).reduce((prose, model) => prose.replaceAll(model, '[模型名已隐去]'), text)
}
async function copy(text: string) {
  try { await navigator.clipboard.writeText(text); studio.flash('已复制提示词') }
  catch { studio.error = '请选中文字后复制。' }
}
onMounted(() => { void studio.refreshTraces().catch(() => { studio.error = '无法读取本机调用记录。' }) })
</script>

<template>
  <section class="workspace-page">
    <header class="workspace-title"><div><h1>实验室</h1><p>查看冻结的写作上下文、供应商用量和同场对照。</p></div></header>
    <nav class="filter-row lab-tabs" aria-label="实验室功能">
      <button class="flat-button" :class="{ selected: tab === 'prompt' }" type="button" @click="tab = 'prompt'">提示词</button>
      <button class="flat-button" :class="{ selected: tab === 'usage' }" type="button" @click="tab = 'usage'">用量账单</button>
      <button class="flat-button" :class="{ selected: tab === 'compare' }" type="button" @click="tab = 'compare'">A/B 盲测</button>
    </nav>
    <div v-if="tab === 'prompt'" class="lab-columns">
      <article class="lab-card">
        <h2>实际发送的提示词</h2>
        <select v-if="studio.tasks.length" v-model="taskId" aria-label="历史生成任务">
          <option disabled value="">最近任务</option>
          <option v-for="task in studio.tasks" :key="task.taskId" :value="task.taskId">
            {{ date.format(new Date(task.frozenContext.capturedAt)) }} · {{ tag(task.taskType) }} · {{ task.frozenContext.chapterTitle }}
          </option>
        </select>
        <template v-if="selectedTask">
          <p class="muted">{{ selectedTask.frozenContext.model }} · {{ selectedTask.state }}</p>
          <pre class="plain-text frozen-prompt">{{ selectedTask.frozenContext.prompt }}</pre>
          <button class="secondary-button" type="button" @click="copy(selectedTask.frozenContext.prompt)">复制 Prompt</button>
          <details><summary>冻结快照</summary><pre class="plain-text">{{ JSON.stringify(selectedTask.frozenContext, null, 2) }}</pre></details>
        </template>
        <p v-else class="muted">完成一次续写后可查看快照。</p>
      </article>
      <article class="lab-card">
        <h2>提示词覆写</h2><p class="muted">作为作者指令加入后续任务。已冻结的任务沿用原版本。</p>
        <textarea v-model="override" rows="8" maxlength="4000" aria-label="作者提示词覆写"></textarea>
        <button class="primary-button" type="button" :disabled="studio.busy" @click="studio.savePrompt(override)">保存新版本</button>
        <div v-if="studio.promptHistory.length" class="prompt-history">
          <button v-for="revision in studio.promptHistory" :key="revision.promptRevisionId" class="history-row" type="button" @click="override = revision.instruction">
            <strong>{{ date.format(new Date(revision.createdAt)) }}</strong><span>{{ revision.instruction || '清空覆写' }}</span>
          </button>
        </div>
      </article>
    </div>
    <div v-else-if="tab === 'usage'" class="lab-card usage-ledger">
      <div class="workspace-actions">
        <h2>调用记录</h2><button class="secondary-button" type="button" @click="studio.refreshTraces()"><RotateCw :size="15" /> 刷新</button>
      </div>
      <table v-if="studio.traces.length">
        <thead><tr><th>时间</th><th>调用</th><th>输入 token</th><th>缓存命中</th><th>输出 token</th><th>耗时</th><th>费用</th><th>状态</th></tr></thead>
        <tbody><tr v-for="trace in studio.traces" :key="trace.traceId">
          <td>{{ date.format(new Date(trace.createdAt)) }}</td><td>{{ tag(trace.stage) }}</td>
          <td>{{ token(trace.promptTokens) }}</td><td>{{ token(trace.cachedTokens) }}</td><td>{{ token(trace.completionTokens) }}</td>
          <td>{{ (trace.latencyMs / 1000).toFixed(1) }} s</td><td>{{ cost(trace) }}</td><td>{{ trace.status }}</td>
        </tr></tbody>
      </table>
      <p v-else class="muted">还没有模型调用。</p>
    </div>
    <template v-else>
      <div class="lab-card ab-control">
        <div class="ab-models">
          <label>模型一<select v-model="modelA" aria-label="模型一"><option value="">选择模型</option>
            <option v-for="item in studio.profiles" :key="item.id" :value="item.id">{{ item.name }}</option></select></label>
          <label>模型二<select v-model="modelB" aria-label="模型二"><option value="">选择模型</option>
            <option v-for="item in studio.profiles" :key="item.id" :value="item.id">{{ item.name }}</option></select></label>
        </div>
        <label>本场写作指令<input v-model="instruction" placeholder="留空则沿用当前场景"></label>
        <button type="button" class="primary-button" :disabled="studio.busy || !modelA || !modelB || modelA === modelB"
          @click="chosenGroup = ''; reveal = false; studio.compareModels(modelA,modelB,instruction)"><Beaker :size="16" /> 开始同场对照</button>
      </div>
      <div v-if="comparisons.length" class="filter-row ab-switch">
        <select v-model="chosenGroup" aria-label="历史 A/B 对照"><option value="">最近一场</option>
          <option v-for="(id, index) in comparisons" :key="id" :value="id">对照 {{ comparisons.length - index }}</option>
        </select>
        <button class="secondary-button" type="button" @click="reveal = !reveal">{{ reveal ? '隐藏模型' : '揭晓模型' }}</button>
      </div>
      <div class="lab-columns">
        <article v-for="task in sides" :key="task.taskId" class="lab-card">
          <h2>版本 {{ task.frozenContext.comparison?.label }}</h2>
          <p class="muted">{{ reveal ? task.frozenContext.model + ' · ' : '' }}{{ task.state }}</p>
          <pre class="plain-text ab-prose">{{ blindProse(task.partialOutput) || '等待正文……' }}</pre>
          <button v-if="task.state === 'completed'" class="primary-button" type="button"
            :disabled="studio.busy || studio.chapter?.chapterId !== task.chapterId || studio.chapter?.currentRevisionId !== task.baseRevisionId"
            @click="studio.acceptDraft(task)"><Check :size="15" /> 采纳此稿</button>
          <button v-if="activeTask(task.state)" class="secondary-button" type="button" @click="studio.cancel(task)">停止</button>
        </article>
      </div>
    </template>
  </section>
</template>

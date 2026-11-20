<script setup lang="ts">
import { computed, nextTick, ref } from 'vue'
import { activeTask } from '../core/contracts/types'
import { characterDiff, parseReplacement } from '../core/narrative/editing'
import { useStudio } from '../stores/studio'

const props = defineProps<{ selection: { start: number; end: number } }>()
const emit = defineEmits<{ locate: [chapter: string, literal: string, offset?: number] }>()
const studio = useStudio()
const instruction = ref('')
const modal = ref<HTMLDialogElement>()
const canon = ref(false)
const latest = computed(() => studio.polishTasks[0])
const proposal = computed(() => {
  const task = latest.value
  if (task?.state !== 'completed' || !task.frozenContext.span) return undefined
  try {
    const replacement = parseReplacement(task.partialOutput)
    return characterDiff(task.frozenContext.span.expected, replacement)
  } catch { return undefined }
})

async function viewDiff() {
  canon.value = false
  await nextTick()
  modal.value?.showModal()
}
async function useDiff() {
  if (!latest.value) return
  await studio.applyPolish(latest.value, canon.value)
  if (!studio.error) modal.value?.close()
}
</script>

<template>
  <section class="review-panel">
    <h3>选区精修</h3>
    <p class="muted">{{ props.selection.end > props.selection.start ? '已选中正文范围' : '在正文选中要精修的文字。' }}</p>
    <input v-model="instruction" aria-label="精修要求" placeholder="精修要求，可留空">
    <button class="secondary-button" type="button" :disabled="studio.busy || studio.dirty
      || props.selection.end <= props.selection.start || !!latest && activeTask(latest.state)"
      @click="studio.requestPolish(props.selection.start, props.selection.end, instruction)">生成精修建议</button>
    <button v-if="latest?.state === 'queued'" class="flat-button" type="button" @click="studio.startQueued(latest)">发送已排队精修</button>
    <p v-else-if="latest && activeTask(latest.state)" class="muted">精修正在生成。</p>
    <p v-else-if="latest?.state === 'completed' && !proposal" class="pending-text">模型未返回可用的精修结果。</p>
    <p v-else-if="latest && studio.writingIssues[latest.taskId]" class="pending-text">{{ studio.writingIssues[latest.taskId] }}</p>
    <button v-if="proposal" class="secondary-button" type="button" @click="viewDiff">查看差异</button>

    <div class="pivot-form">
      <h3>文学诊断</h3>
      <button class="secondary-button" type="button" :disabled="studio.busy || studio.dirty || !studio.chapter?.currentRevisionId
        || studio.criticTasks.some(task => activeTask(task.state))" @click="studio.requestCritic()">请主编挑刺</button>
      <div v-for="task in studio.criticTasks.filter(item => item.state === 'queued')" :key="task.taskId" class="memory-actions">
        <button class="flat-button" type="button" @click="studio.startQueued(task)">发送已排队诊断</button>
        <button class="flat-button" type="button" @click="studio.cancel(task)">取消</button>
      </div>
      <p v-if="studio.criticTasks.some(task => task.state === 'running' || task.state === 'streaming')" class="muted">主编正在审读。</p>
      <p v-else-if="studio.criticTasks[0]?.state === 'completed' && !studio.criticFindings.length" class="muted">本次没有可定位的建议。</p>
      <article v-for="finding in studio.criticFindings" :key="finding.anchor.startOffset + finding.message" class="review-finding">
        <strong>{{ finding.message }}</strong>
        <button type="button" class="memory-row" @click="emit('locate', studio.chapterId, finding.anchor.quote, finding.anchor.startOffset)">{{ finding.anchor.quote }}</button>
        <p>{{ finding.suggestion }}</p>
      </article>
    </div>

    <div v-if="studio.editImpact?.revisionId === studio.chapter?.currentRevisionId && studio.editImpact" class="pivot-form">
      <h3>可能受影响的后续章节</h3>
      <p v-if="!studio.editImpact.terms.length" class="muted">保存时可填写道具或人物关键词，也可先在记忆库登记设定。</p>
      <p v-else-if="!studio.editImpact.hits.length" class="muted">后续正文没有匹配到 {{ studio.editImpact.terms.join('、') }}。</p>
      <button v-for="hit in studio.editImpact.hits" :key="hit.chapterId" type="button" class="memory-row"
        @click="emit('locate', hit.chapterId, studio.editImpact?.terms.find(term => hit.excerpt.includes(term)) ?? '')">
        <strong>第 {{ hit.chapterOrder }} 章 · {{ hit.title }}</strong><small>{{ hit.excerpt }}</small>
      </button>
    </div>

    <dialog ref="modal" class="diff-dialog" aria-label="精修差异">
      <div class="memory-form">
        <h2>精修差异</h2>
        <p class="diff-legend"><span class="diff-removed">删去</span><span class="diff-added">加入</span></p>
        <div class="diff-content"><template v-for="(item, index) in proposal" :key="index">
          <del v-if="item.type === 'removed'" class="diff-removed">{{ item.text }}</del>
          <ins v-else-if="item.type === 'added'" class="diff-added">{{ item.text }}</ins>
          <span v-else>{{ item.text }}</span>
        </template></div>
        <label v-if="latest?.baseRevisionId === studio.chapter?.currentRevisionId" class="check-line">
          <input v-model="canon" type="checkbox"> 如涉及已确认事实，我确认这项变更</label>
        <p v-else class="pending-text">正文已换版。请重新选区生成。</p>
        <p v-if="studio.error" class="pending-text" role="alert">{{ studio.error }}</p>
        <div class="memory-actions">
          <button class="primary-button" type="button" :disabled="studio.busy || studio.dirty
            || latest?.baseRevisionId !== studio.chapter?.currentRevisionId" @click="useDiff">应用到正文</button>
          <button class="flat-button" type="button" @click="modal?.close()">关闭</button>
        </div>
      </div>
    </dialog>
  </section>
</template>

<script setup lang="ts">
import { computed, ref, toRaw } from 'vue'
import { Milestone, Route } from 'lucide-vue-next'
import { desktop } from '../core/db/desktop'
import { errorMessage } from '../core/contracts/types'
import type { PivotPreview, StageArc } from '../core/narrative/editing'
import { useStudio } from '../stores/studio'

const studio = useStudio()
const empty = (): StageArc => ({ arcId: '', title: '', roughGoal: '', expectedChapters: 12, milestones: [],
  pivotInstruction: '', pivotRemaining: 0, pivotStartOrder: 0, status: 'active', version: 0, writerVisible: false })
const editor = ref(empty())
const pivoting = ref(false)
const nextGoal = ref('')
const method = ref('')
const chapters = ref(0)
const preview = ref<PivotPreview>()
const assigned = computed(() => studio.chapters.filter(chapter => chapter.arcId === editor.value.arcId))
const completed = computed(() => assigned.value.filter(chapter => chapter.currentRevisionId).length)

function select(arc?: StageArc) {
  editor.value = arc ? structuredClone(toRaw(arc)) : empty()
  pivoting.value = false
  preview.value = undefined
}
async function save() {
  const id = editor.value.arcId
  await studio.saveArc(editor.value)
  if (!studio.error) select(studio.arcs.find(item => item.arcId === id) ?? studio.arcs.at(-1))
}
function startPivot() {
  pivoting.value = true
  preview.value = undefined
  nextGoal.value = editor.value.roughGoal
  method.value = ''
}
function addMilestone() { editor.value.milestones.push({ id: crypto.randomUUID(), text: '', done: false }) }
async function checkPivot() {
  try {
    const result = await desktop.previewPivot(editor.value.arcId)
    chapters.value = result.suggested
    preview.value = result
  } catch (error) { studio.error = errorMessage(error) }
}
async function commit() {
  if (!preview.value) return
  const id = editor.value.arcId
  await studio.applyPivot({ arcId: id, version: preview.value.version, nextGoal: nextGoal.value, instruction: method.value,
    transitionChapters: chapters.value, noteIds: preview.value.notes.map(note => note.noteId) })
  if (!studio.error) select(studio.arcs.find(item => item.arcId === id))
}
</script>

<template>
  <main class="workspace-page">
    <header class="workspace-intro"><h1><Route :size="25" /> 大纲</h1></header>
    <div class="memory-grid">
      <section class="memory-list" aria-label="阶段列表">
        <button class="secondary-button" type="button" @click="select()">新建阶段</button>
        <button v-for="arc in studio.arcs" :key="arc.arcId" type="button" class="memory-row"
          :class="{ selected: arc.arcId === editor.arcId }" @click="select(arc)">
          <strong>{{ arc.title }}</strong>
          <small>{{ arc.status === 'completed' ? '已完成' : '进行中' }} · {{ arc.milestones.filter(step => step.done).length }}/{{ arc.milestones.length }} 个路标</small>
        </button>
        <label v-if="studio.chapter" class="workspace-field memory-form">当前章所属阶段
          <select :value="studio.chapter.arcId ?? ''" @change="studio.assignArc(studio.chapterId, ($event.target as HTMLSelectElement).value || null)">
            <option value="">未分配</option>
            <option v-for="arc in studio.arcs" :key="arc.arcId" :value="arc.arcId">{{ arc.title }}</option>
          </select>
        </label>
      </section>
      <section class="workspace-field memory-form">
        <form class="memory-form" @submit.prevent="save">
          <h2>{{ editor.arcId ? '阶段设置' : '新建阶段' }}</h2>
          <label>阶段名称<input v-model.trim="editor.title" required maxlength="120"></label>
          <label>粗线条目标<textarea v-model="editor.roughGoal" required rows="4" :readonly="!!editor.arcId"></textarea></label>
          <label>预计章数<input v-model.number="editor.expectedChapters" type="number" min="1" max="999" required></label>
          <p v-if="editor.arcId" class="muted">已保存正文 {{ completed }} / {{ editor.expectedChapters }} 章 · 已分配 {{ assigned.length }} 章</p>
          <label>阶段状态<select v-model="editor.status"><option value="active">进行中</option><option value="completed">已完成</option></select></label>
          <label class="check-line"><input v-model="editor.writerVisible" type="checkbox"> 将目标、未完成路标和过渡说明提供给主笔</label>
          <h3><Milestone :size="16" /> 路标</h3>
          <div v-for="(step, index) in editor.milestones" :key="step.id" class="milestone-row">
            <input v-model="step.done" type="checkbox" :aria-label="'完成路标 ' + (index + 1)">
            <input v-model="step.text" :aria-label="'路标 ' + (index + 1)" maxlength="300" required>
            <button type="button" class="flat-button" @click="editor.milestones.splice(index, 1)">移除</button>
          </div>
          <button type="button" class="flat-button" :disabled="editor.milestones.length >= 40"
            @click="addMilestone">新增路标</button>
          <div class="memory-actions">
            <button class="primary-button" type="submit" :disabled="studio.busy">保存阶段</button>
            <button v-if="editor.arcId && !pivoting" class="secondary-button" type="button" @click="startPivot">调整目标</button>
          </div>
        </form>
        <form v-if="pivoting" class="memory-form pivot-form" @submit.prevent="preview ? commit() : checkPivot()">
          <h3>改纲预览</h3>
          <label>新目标<textarea v-model="nextGoal" rows="4" required @input="preview = undefined"></textarea></label>
          <template v-if="preview">
            <p class="muted">原目标　{{ preview.previousGoal }}</p>
            <h4>需要检查的旧伏笔与备注</h4>
            <ul v-if="preview.notes.length">
              <li v-for="note in preview.notes" :key="note.noteId">{{ studio.chapters.find(chapter => chapter.chapterId === note.chapterId)?.title }} · {{ note.body }}</li>
            </ul>
            <p v-else class="muted">该阶段暂无未回收的备注。</p>
            <label>过渡章数（建议 {{ preview.suggested }}）<input v-model.number="chapters" type="number" required min="0" max="999"></label>
            <label v-if="chapters > 0">过渡安排<textarea v-model="method" rows="3" required></textarea></label>
          </template>
          <div class="memory-actions">
            <button type="submit" class="primary-button" :disabled="studio.busy">{{ preview ? '确认改纲' : '检查影响' }}</button>
            <button class="flat-button" type="button" @click="pivoting = false; preview = undefined">取消</button>
          </div>
        </form>
      </section>
    </div>
  </main>
</template>

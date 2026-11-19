<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { BookOpen, Import, ScrollText, Sparkles } from 'lucide-vue-next'
import { activeTask, errorMessage, type Pack } from '../core/contracts/types'
import { parseCartridge, parseSkill, unwrapSkillDraft } from '../core/skills/pack'
import { useStudio } from '../stores/studio'

const studio = useStudio()
const input = ref<HTMLInputElement>()
const focusId = ref('')
const tab = ref<'shelf' | 'extract'>('shelf')
const excerpt = ref('')
const draft = ref('')
const filter = ref<'all' | Pack['slot']>('all')
const error = ref('')
const focused = computed(() => studio.styles.packs.find(item => item.itemId === focusId.value))
const displayed = computed(() => studio.styles.packs.filter(item => filter.value === 'all' || item.slot === filter.value))
const latest = computed(() => studio.tasks.find(task => task.taskType === 'skill_extract'))
const counts = computed(() => Array.from(excerpt.value.trim()).length)
const selectedBase = computed(() => studio.styles.packs.find(pack => pack.itemId === studio.styles.activeBaseSkillId))
const selectedCartridge = computed(() => studio.styles.packs.find(pack => pack.itemId === studio.styles.activePackId))

watch(() => latest.value?.state, state => {
  if (state === 'completed' && latest.value) draft.value = unwrapSkillDraft(latest.value.partialOutput)
}, { immediate: true })

async function importFiles(files: FileList | null) {
  if (!files?.length) return
  error.value = ''
  try {
    const packs: Pack[] = []
    if (files.length > 20) throw new Error('一次最多导入 20 个文件。')
    for (const file of Array.from(files)) {
      if (file.size > 150000) throw new Error(file.name + ' 超出配方文件上限。')
      if (file.name.toLowerCase().endsWith('.md')) packs.push(parseSkill(await file.text()))
      else if (file.name.toLowerCase().endsWith('.json')) packs.push(parseCartridge(await file.text()))
      else throw new Error('请选择 .md Writing Skill 或 .json 流派卡带。')
    }
    await studio.importPacks(packs)
    focusId.value = packs[0]?.itemId ?? ''
    tab.value = 'shelf'
    filter.value = 'all'
  } catch (reason) { error.value = errorMessage(reason) }
  finally { if (input.value) input.value.value = '' }
}
function attach(pack: Pack) {
  if (pack.slot === 'base_style') void studio.setStyle(
    studio.styles.activeBaseSkillId === pack.itemId ? null : pack.itemId, studio.styles.activePackId,
  )
  if (pack.slot === 'cartridge') void studio.setStyle(studio.styles.activeBaseSkillId,
    studio.styles.activePackId === pack.itemId ? null : pack.itemId)
}
async function saveDraft() {
  error.value = ''
  try {
    const pack = parseSkill(draft.value)
    await studio.importPacks([pack])
    if (!studio.error) { focusId.value = pack.itemId; tab.value = 'shelf'; filter.value = 'all' }
  } catch (reason) { error.value = errorMessage(reason) }
}
</script>

<template>
  <section class="workspace-page" @dragover.prevent @drop.prevent="importFiles($event.dataTransfer?.files ?? null)">
    <header class="workspace-title">
      <div><h1>卡带与配方</h1><p>底色和专项技法可组合，题材卡带单独选择。</p></div>
      <div class="workspace-actions">
        <input ref="input" class="sr-only" type="file" accept=".md,.json" multiple @change="importFiles(($event.target as HTMLInputElement).files)">
        <button class="secondary-button" type="button" @click="input?.click()"><Import :size="16" /> 导入文件</button>
        <button class="primary-button" type="button" @click="tab = tab === 'extract' ? 'shelf' : 'extract'"><Sparkles :size="16" />
          {{ tab === 'extract' ? '返回货架' : '从样章生成 Skill' }}</button>
      </div>
    </header>
    <p v-if="error" class="form-error workspace-error" role="alert">{{ error }}</p>
    <div v-if="tab === 'shelf'" class="pack-layout">
      <div>
        <div class="filter-row">
          <button v-for="([name, value]) in [['全部','all'],['底色','base_style'],['专项','technique'],['流派','cartridge']] as const"
            :key="value" class="flat-button" type="button" :class="{ selected: filter === value }" @click="filter = value">{{ name }}</button>
        </div>
        <div class="pack-grid">
          <button v-for="pack in displayed" :key="pack.itemId" class="pack-card" type="button" :class="{ selected: pack.itemId === focusId }"
            @click="focusId = pack.itemId">
            <span class="pack-glyph"><BookOpen v-if="pack.slot === 'cartridge'" :size="19" /><ScrollText v-else :size="19" /></span>
            <strong>{{ pack.name }}</strong>
            <span>{{ pack.slot === 'base_style' ? '底色' : pack.slot === 'technique' ? '专项技法' : '流派卡带' }} · {{ pack.version }}</span>
            <small v-if="!pack.enabled">已停用</small>
            <small v-else-if="selectedBase?.itemId === pack.itemId || selectedCartridge?.itemId === pack.itemId">当前使用</small>
          </button>
        </div>
      </div>
      <article class="pack-detail">
        <template v-if="focused">
          <h2>{{ focused.name }}</h2>
          <div class="pack-detail-actions">
            <button v-if="focused.slot !== 'technique'" class="primary-button" type="button" :disabled="!focused.enabled || studio.busy" @click="attach(focused)">
              {{ focused.itemId === selectedBase?.itemId || focused.itemId === selectedCartridge?.itemId ? '取消当前选择' : '设为当前使用' }}
            </button>
            <button class="secondary-button" type="button" :disabled="studio.busy" @click="studio.togglePack(focused)">{{ focused.enabled ? '停用' : '启用' }}</button>
          </div>
          <pre class="plain-text">{{ focused.body }}</pre>
          <ol v-if="focused.metadata.stages?.length" class="cartridge-stage">
            <li v-for="stage in focused.metadata.stages" :key="stage">{{ stage }}</li>
          </ol>
        </template>
        <p v-else class="muted">选择一张卡片查看规则与示例，也可拖入本机配方文件。</p>
      </article>
    </div>
    <div v-else class="extract-grid">
      <label class="workspace-field">原文样章 <span>{{ counts }} / 3000 字</span><textarea v-model="excerpt" rows="17" placeholder="粘贴 1500 至 3000 字样章"></textarea>
        <button class="primary-button" type="button" :disabled="studio.busy || counts < 1500 || counts > 3000 || !!(latest && activeTask(latest.state))"
          @click="studio.runSkillExtraction(excerpt)"><Sparkles :size="16" /> 提取技法</button>
      </label>
      <label class="workspace-field">YAML + Markdown 草稿
        <textarea v-model="draft" rows="17" spellcheck="false" placeholder="可审核并编辑生成的 Skill"></textarea>
        <button class="secondary-button" type="button" :disabled="!draft.trim() || studio.busy" @click="saveDraft">校验并保存</button>
        <span v-if="latest && activeTask(latest.state)" class="muted">正在生成 Skill 草稿。</span>
        <span v-else-if="latest?.state === 'outcome_unknown'" class="muted">远端结果未知。收到的内容仍保存在实验室。</span>
      </label>
    </div>
  </section>
</template>

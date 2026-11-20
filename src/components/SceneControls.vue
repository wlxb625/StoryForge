<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'
import { Settings2 } from 'lucide-vue-next'
import type { SceneIntent } from '../core/contracts/types'
import { noteIsDue } from '../core/narrative/memory'
import { repeatedActions } from '../core/narrative/intent'
import { useStudio } from '../stores/studio'

const studio = useStudio()
const editingExisting = ref(true)
const form = reactive({ title: '', time: '', location: '', cast: '', conflict: '', povCharacterId: '', requiresPreviousMemory: false })
const fields = [
  ['goal', '戏剧目的'], ['hiddenGoal', '潜在目的'], ['action', '表层行为'],
  ['emotion', '内在情绪'], ['dialogue', '对话状态'], ['pace', '节奏变化'], ['pov', '视角限制'],
] as const
const intent = reactive<Partial<SceneIntent>>({})
const chosen = ref<string[]>([])
const stage = ref('')
const cartridge = computed(() => studio.sceneStyles.cartridge)
const techniques = computed(() => studio.styles.packs.filter(pack => pack.slot === 'technique' && pack.enabled))
const viewpoint = computed(() => studio.memory.canon.find(item => item.entryId === studio.scene?.details.povCharacterId))
const propsInScene = computed(() => studio.memory.canon.filter(item => item.category === 'prop' &&
  [item.name, ...item.aliases].some(name => (studio.scene?.details.conflict ?? '').includes(name))))
const due = computed(() => studio.memory.notes.filter(note => noteIsDue(note, studio.chapter?.chapterOrder ?? 0)))
const repeated = computed(() => repeatedActions(studio.acceptedText))

function load() {
  const scene = editingExisting.value ? studio.scene : undefined
  Object.assign(form, { title: scene?.title ?? '', time: scene?.details.time ?? '', location: scene?.details.location ?? '',
    cast: scene?.details.cast ?? '', conflict: scene?.details.conflict ?? '', povCharacterId: scene?.details.povCharacterId ?? '',
    requiresPreviousMemory: scene?.details.requiresPreviousMemory ?? false })
  for (const [key] of fields) intent[key] = scene?.details.intent?.[key] ?? ''
  intent.humor = scene?.details.intent?.humor
  chosen.value = [...(scene?.details.techniqueIds ?? [])]
  stage.value = scene?.details.cartridgeStage?.packId === cartridge.value?.itemId ? scene?.details.cartridgeStage?.stage ?? '' : ''
}
watch(() => [studio.scene?.sceneId, studio.scene?.version, cartridge.value?.itemId], () => { editingExisting.value = true; load() }, { immediate: true })

function change(event: Event) {
  const id = (event.target as HTMLSelectElement).value
  void studio.selectScene(id || null)
}
function save() {
  const current = editingExisting.value ? studio.scene : undefined
  if (!form.title.trim()) return
  void studio.saveScene({ sceneId: current?.sceneId, version: current?.version, title: form.title,
    details: { time: form.time, location: form.location, cast: form.cast, conflict: form.conflict,
      intent: { ...intent }, techniqueIds: chosen.value, povCharacterId: form.povCharacterId || undefined,
      cartridgeStage: stage.value && cartridge.value ? { packId: cartridge.value.itemId, stage: stage.value } : undefined,
      requiresPreviousMemory: form.requiresPreviousMemory },
  })
}
function toggle(id: string) {
  if (chosen.value.includes(id)) chosen.value = chosen.value.filter(item => item !== id)
  else if (chosen.value.length < 2) chosen.value = [...chosen.value, id]
}
</script>

<template>
  <div v-if="studio.chapter" class="scene-block">
    <div class="scene-caps"><span>{{ studio.scene?.title ?? '自动场景' }}</span><span v-if="studio.sceneStyles.cartridge">{{ studio.sceneStyles.cartridge.name }}</span>
      <span v-if="viewpoint">视角 · {{ viewpoint.name }}</span><span v-for="prop in propsInScene" :key="prop.entryId">道具 · {{ prop.name }}</span>
      <span v-for="pack in studio.sceneStyles.skills" :key="pack.itemId">{{ pack.name }}</span>
    </div>
    <details v-if="due.length || repeated.length" class="scene-settings">
      <summary>写作提醒</summary>
      <p v-for="note in due" :key="note.noteId" class="muted">可考虑回收 · {{ note.body }}</p>
      <p v-if="repeated.length" class="muted">近文有重复动作 · {{ repeated.join('、') }}</p>
    </details>
    <details class="scene-settings">
      <summary><Settings2 :size="14" /> 场景微调</summary>
      <div class="scene-form">
        <label>当前场景
          <select :value="studio.scene?.sceneId ?? ''" @change="change">
            <option value="">自动</option>
            <option v-for="scene in studio.scenes" :key="scene.sceneId" :value="scene.sceneId">{{ scene.title }}</option>
          </select>
        </label>
        <button v-if="studio.scene" class="secondary-button" type="button" :disabled="studio.busy" @click="studio.completeScene()">完成并审查本场</button>
        <button v-if="studio.scene" class="flat-button" type="button" @click="editingExisting = false; load()">另建场景</button>
        <form @submit.prevent="save">
          <label>场景名称<input v-model="form.title" required maxlength="120" placeholder="名称"></label>
          <label>时间<input v-model="form.time" placeholder="沿用前情"></label>
          <label>地点<input v-model="form.location" placeholder="沿用前情"></label>
          <label>在场人物<input v-model="form.cast" placeholder="用顿号分隔"></label>
          <label>视角人物
            <select v-model="form.povCharacterId">
              <option value="">未指定</option>
              <option v-for="person in studio.memory.canon.filter(item => item.category === 'character')" :key="person.entryId" :value="person.entryId">{{ person.name }}</option>
            </select>
          </label>
          <label>核心矛盾<textarea v-model="form.conflict" rows="2"></textarea></label>
          <label v-if="cartridge?.metadata.stages?.length">卡带路标
            <select v-model="stage"><option value="">跟随同名大纲阶段</option>
              <option v-for="item in cartridge.metadata.stages" :key="item">{{ item }}</option>
            </select>
          </label>
          <label class="check-line"><input type="checkbox" v-model="form.requiresPreviousMemory"> 依赖上一章同步记忆</label>
          <label v-for="[key,label] in fields" :key="key">{{ label }}<input v-model="intent[key]" :placeholder="studio.sceneIntent[key]"></label>
          <label>幽默许可
            <select v-model="intent.humor"><option :value="undefined">自动（{{ studio.sceneIntent.humor }}）</option>
              <option>允许</option><option>克制</option><option>禁止</option>
            </select>
          </label>
          <fieldset><legend>专项技法（最多两项）</legend>
            <label v-for="skill in techniques" :key="skill.itemId" class="check-line">
              <input type="checkbox" :checked="chosen.includes(skill.itemId)" :disabled="chosen.length >= 2 && !chosen.includes(skill.itemId)"
                @change="toggle(skill.itemId)">{{ skill.name }}
            </label>
          </fieldset>
          <button type="submit" class="primary-button" :disabled="studio.busy">保存场景</button>
        </form>
      </div>
    </details>
  </div>
</template>

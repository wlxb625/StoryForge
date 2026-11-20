<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'
import { Settings2 } from 'lucide-vue-next'
import type { SceneIntent } from '../core/contracts/types'
import { useStudio } from '../stores/studio'

const studio = useStudio()
const editingExisting = ref(true)
const form = reactive({ title: '', time: '', location: '', cast: '', conflict: '', povCharacterId: '' })
const fields = [
  ['goal', '戏剧目的'], ['hiddenGoal', '潜在目的'], ['action', '表层行为'],
  ['emotion', '内在情绪'], ['dialogue', '对话状态'], ['pace', '节奏变化'], ['pov', '视角限制'],
] as const
const intent = reactive<Partial<SceneIntent>>({})
const chosen = ref<string[]>([])
const techniques = computed(() => studio.styles.packs.filter(pack => pack.slot === 'technique' && pack.enabled))

function load() {
  const scene = editingExisting.value ? studio.scene : undefined
  Object.assign(form, { title: scene?.title ?? '', time: scene?.details.time ?? '', location: scene?.details.location ?? '',
    cast: scene?.details.cast ?? '', conflict: scene?.details.conflict ?? '', povCharacterId: scene?.details.povCharacterId ?? '' })
  for (const [key] of fields) intent[key] = scene?.details.intent?.[key] ?? ''
  intent.humor = scene?.details.intent?.humor
  chosen.value = [...(scene?.details.techniqueIds ?? [])]
}
watch(() => [studio.scene?.sceneId, studio.scene?.version], () => { editingExisting.value = true; load() }, { immediate: true })

function change(event: Event) {
  const id = (event.target as HTMLSelectElement).value
  void studio.selectScene(id || null)
}
function save() {
  const current = editingExisting.value ? studio.scene : undefined
  if (!form.title.trim()) return
  void studio.saveScene({ sceneId: current?.sceneId, version: current?.version, title: form.title,
    details: { time: form.time, location: form.location, cast: form.cast, conflict: form.conflict,
      intent: { ...intent }, techniqueIds: chosen.value, povCharacterId: form.povCharacterId || undefined },
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
      <span v-for="pack in studio.sceneStyles.skills" :key="pack.itemId">{{ pack.name }}</span>
    </div>
    <details class="scene-settings">
      <summary><Settings2 :size="14" /> 场景微调</summary>
      <div class="scene-form">
        <label>本章场景
          <select :value="studio.scene?.sceneId ?? ''" @change="change">
            <option value="">自动</option>
            <option v-for="scene in studio.scenes" :key="scene.sceneId" :value="scene.sceneId">{{ scene.title }}</option>
          </select>
        </label>
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
